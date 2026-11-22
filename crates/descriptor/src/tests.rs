use serde_json::{Value, json};

use super::*;

/// A full descriptor of an `orders` plugin, consistent.
fn orders() -> Value {
    json!({
      "schema": 1,
      "id": "orders",
      "version": "1.2.0",
      "title": "Orders",
      "description": "Orders of catalog products by customers.",
      "roles": [{ "name": "clerk", "title": "Order clerk", "description": "", "strict": false, "default_for": "member" },
                { "name": "manager", "title": "Manager", "description": "", "strict": true, "default_for": "none" }],
      "scopes": [{ "name": "read", "title": "Read orders", "description": "", "sensitive": false }],
      "capabilities": [{ "name": "push:send", "reason": "Reminds clerks of their unpaid orders." }],
      "permissions": ["members.email"],
      "agents": [{ "name": "sync", "roles": ["clerk"] }],
      "dependencies": [{ "plugin": "customers", "calls": ["GetCustomer"] }],
      "enums": [{ "name": "OrderStatus", "values": ["placed", "paid", "shipped", "cancelled"] }],
      "messages": [
        { "name": "PlaceOrderRequest", "kind": "message", "description": "",
          "fields": [
            { "name": "customer_id", "number": 1, "type": "ref:customers.customer", "repeated": false, "optional": false, "description": "" },
            { "name": "lines", "number": 2, "type": "message:Line", "repeated": true, "optional": false, "description": "" }
          ] },
        { "name": "Line", "kind": "message", "description": "",
          "fields": [{ "name": "sku", "number": 1, "type": "string", "repeated": false, "optional": false, "description": "" },
                     { "name": "quantity", "number": 2, "type": "int32", "repeated": false, "optional": false, "description": "" }] }
      ],
      "entities": [{ "name": "order", "message": "Order", "key": "uuid", "label": "number",
                     "soft_delete": false, "version": true, "append_only": false,
                     "fields": [
                       { "name": "number", "number": 1, "type": "string", "repeated": false, "optional": false, "description": "" },
                       { "name": "status", "number": 2, "type": "enum:OrderStatus", "repeated": false, "optional": false, "description": "" },
                       { "name": "created_by", "number": 3, "type": "ref:platform.user", "repeated": false, "optional": false, "description": "" },
                       { "name": "parent", "number": 4, "type": "ref:orders.order", "repeated": false, "optional": true, "description": "" },
                       { "name": "total", "number": 5, "type": "decimal", "repeated": false, "optional": false, "description": "" },
                       { "name": "id", "number": 6, "type": "ref:orders.order", "repeated": false, "optional": false, "description": "" }
                     ] }],
      "endpoints": [{ "name": "PlaceOrder", "input": "PlaceOrderRequest", "output": "Order", "description": "",
                      "auth": "user", "roles": ["clerk", "manager"], "scopes": ["write"],
                      "idempotency": "optional", "timeout_ms": 30000 },
                    { "name": "ListOrders", "input": "Line", "output": "Order", "description": "",
                      "auth": "user", "roles": [], "scopes": ["read"], "idempotency": "off" }]
    })
}

fn errors(v: &Value) -> Vec<Error> {
    match check(v.to_string().as_bytes()) {
        Ok(_) => vec![],
        Err(e) => e,
    }
}

/// The codes of the errors of `v` after `f` changed it.
fn codes_after(f: impl FnOnce(&mut Value)) -> Vec<(&'static str, String)> {
    let mut v = orders();
    f(&mut v);
    errors(&v).into_iter().map(|e| (e.code, e.path)).collect()
}

fn has(errs: &[(&'static str, String)], code: &str, path: &str) -> bool {
    errs.iter().any(|(c, p)| *c == code && p == path)
}

macro_rules! expect {
    ($code:literal, $path:literal, $f:expr) => {{
        let errs = codes_after($f);
        assert!(
            has(&errs, $code, $path),
            "expected {} at {}, got {:?}",
            $code,
            $path,
            errs
        );
    }};
}

#[test]
fn the_example_is_valid() {
    let c = check(orders().to_string().as_bytes()).unwrap();
    assert_eq!(c.descriptor.id, "orders");
    assert_eq!(c.descriptor.endpoints[0].idempotency, Idempotency::Optional);
    assert_eq!(c.descriptor.entities[0].key, "uuid");
}

#[test]
fn minimal_descriptor_takes_defaults() {
    let c = check(br#"{"schema":1,"id":"hello","version":"0.1.0"}"#).unwrap();
    assert!(c.descriptor.endpoints.is_empty());
    assert_eq!(
        c.canonical,
        r#"{"id":"hello","schema":1,"version":"0.1.0"}"#
    );
}

#[test]
fn schema_errors() {
    let e = check(b"not json").unwrap_err();
    assert_eq!(e[0].code, "E_SCHEMA");
    let e = check(br#"{"schema":2,"id":"hello","version":"0.1.0"}"#).unwrap_err();
    assert_eq!((e[0].code, e[0].path.as_str()), ("E_SCHEMA", "schema"));
    let e = check(br#"{"id":"hello","version":"0.1.0"}"#).unwrap_err();
    assert_eq!(e[0].code, "E_SCHEMA");
    expect!(
        "E_SCHEMA",
        "endpoints[0].auth",
        |v| v["endpoints"][0]["auth"] = json!("nobody")
    );
    expect!(
        "E_SCHEMA",
        "roles[0].default_for",
        |v| v["roles"][0]["default_for"] = json!("all")
    );
    expect!(
        "E_SCHEMA",
        "messages[0].kind",
        |v| v["messages"][0]["kind"] = json!("table")
    );
    expect!("E_SCHEMA", "endpoints[0].idempotency", |v| v["endpoints"]
        [0]["idempotency"] =
        json!("maybe"));
    // unknown members are refused, so nothing an SDK prints is silently lost
    expect!("E_SCHEMA", "endpoints[0].runtime", |v| v["endpoints"][0]
        ["runtime"] =
        json!("rust"));
    expect!("E_SCHEMA", "runtime", |v| v["runtime"] = json!("rust"));
    expect!(
        "E_SCHEMA",
        "messages[PlaceOrderRequest].fields[customer_id].number",
        |v| v["messages"][0]["fields"][0]["number"] = json!(0)
    );
}

#[test]
fn plugin_id_and_version() {
    expect!("E_PLUGIN_ID", "id", |v| v["id"] = json!("My-Orders"));
    expect!("E_PLUGIN_ID", "id", |v| v["id"] = json!("o"));
    expect!("E_PLUGIN_ID_RESERVED", "id", |v| v["id"] =
        json!("platform"));
    expect!("E_PLUGIN_ID_RESERVED", "id", |v| v["id"] = json!("pgadmin"));
    expect!("E_PLUGIN_VERSION", "version", |v| v["version"] =
        json!("1.2"));
    assert_eq!(
        plugin_id_errors("api", "id")[0].code,
        "E_PLUGIN_ID_RESERVED"
    );
    assert!(plugin_id_errors("crm", "id").is_empty());
}

#[test]
fn roles_and_scopes() {
    expect!(
        "E_ROLE_NAME",
        "roles[Clerk].name",
        |v| v["roles"][0]["name"] = json!("Clerk")
    );
    expect!("E_ROLE_DUP", "roles[clerk]", |v| v["roles"][1]["name"] =
        json!("clerk"));
    expect!("E_SCOPE_NAME", "scopes[deals.read].name", |v| v["scopes"]
        [0]["name"] =
        json!("deals.read"));
    expect!(
        "E_SCOPE_NAME",
        "scopes[use].name",
        |v| v["scopes"][0]["name"] = json!("use")
    );
    expect!("E_SCOPE_DUP", "scopes[read]", |v| v["scopes"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "read" })));
}

#[test]
fn capabilities() {
    expect!(
        "E_CAPABILITY_UNKNOWN",
        "capabilities[http:egress].name",
        |v| v["capabilities"][0]["name"] = json!("http:egress")
    );
    expect!(
        "E_CAPABILITY_REASON",
        "capabilities[push:send].reason",
        |v| v["capabilities"][0]["reason"] = json!(" ")
    );
}

#[test]
fn notification_kinds() {
    let kinds = json!([
        { "name": "mention", "title": "Mentions", "description": "Someone wrote @you", "digest": true },
        { "name": "thread_reply", "title": "Replies in threads you follow", "tab": "watching" }
    ]);
    let mut v = orders();
    v["notifications"] = kinds.clone();
    let c = check(v.to_string().as_bytes()).unwrap();
    let k = &c.descriptor.notifications;
    assert_eq!((k[0].tab, k[0].digest), (NotificationTab::ForYou, true));
    assert_eq!((k[1].tab, k[1].digest), (NotificationTab::Watching, false));
    assert_eq!(k[1].description, "");
    // the defaults are read back the same, and a plugin without kinds writes none
    assert!(c.canonical.contains(r#""tab":"watching""#));
    assert!(
        !check(orders().to_string().as_bytes())
            .unwrap()
            .canonical
            .contains("notifications")
    );

    let with = |f: fn(&mut Value)| {
        let k = kinds.clone();
        move |v: &mut Value| {
            v["notifications"] = k;
            f(v)
        }
    };
    expect!(
        "E_NOTIFY_CAPABILITY",
        "notifications",
        with(|v| v["capabilities"] = json!([]))
    );
    expect!(
        "E_NOTIFY_KIND_NAME",
        "notifications[Mention].name",
        with(|v| v["notifications"][0]["name"] = json!("Mention"))
    );
    expect!(
        "E_NOTIFY_KIND_NAME",
        "notifications[mention]",
        with(|v| v["notifications"][1]["name"] = json!("mention"))
    );
    expect!(
        "E_NOTIFY_KIND_NAME",
        "notifications[aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa].name",
        with(|v| v["notifications"][0]["name"] = json!("a".repeat(33)))
    );
    expect!(
        "E_NOTIFY_KIND_RESERVED",
        "notifications[approval].name",
        with(|v| v["notifications"][0]["name"] = json!("approval"))
    );
    expect!(
        "E_NOTIFY_KIND_RESERVED",
        "notifications[run_failed].name",
        with(|v| v["notifications"][0]["name"] = json!("run_failed"))
    );
    expect!(
        "E_NOTIFY_KIND_TEXT",
        "notifications[mention].title",
        with(|v| v["notifications"][0]["title"] = json!(" "))
    );
    expect!(
        "E_NOTIFY_KIND_TEXT",
        "notifications[mention].title",
        with(|v| v["notifications"][0]["title"] = json!("x".repeat(41)))
    );
    expect!(
        "E_NOTIFY_KIND_TEXT",
        "notifications[mention].description",
        with(|v| v["notifications"][0]["description"] = json!("x".repeat(121)))
    );
    expect!(
        "E_NOTIFY_KINDS",
        "notifications",
        with(|v| {
            v["notifications"] = (0..21)
                .map(|i| json!({ "name": format!("kind_{i}"), "title": "A kind" }))
                .collect()
        })
    );
    // an unknown tab is a schema error
    let mut v = orders();
    v["notifications"] = json!([{ "name": "mention", "title": "Mentions", "tab": "elsewhere" }]);
    assert!(check(v.to_string().as_bytes()).is_err());
}

#[test]
fn agents() {
    expect!(
        "E_AGENT_NAME",
        "agents[Sync].name",
        |v| v["agents"][0]["name"] = json!("Sync")
    );
    expect!(
        "E_AGENT_ROLE_UNKNOWN",
        "agents[sync].roles[0]",
        |v| v["agents"][0]["roles"][0] = json!("workspace.admin")
    );
    expect!("E_AGENT_ENGINE", "agents[sync].engine", |v| v["agents"]
        [0]["engine"] =
        json!("gpt"));
    expect!("E_AGENT_MODEL", "agents[sync].model", |v| v["agents"][0]
        ["model"] =
        json!("claude opus"));
    expect!(
        "E_AGENT_ICON",
        "agents[sync].icon",
        |v| v["agents"][0]["icon"] = json!("Git Pull Request")
    );
    for seconds in [59, 6 * 3600 + 1] {
        expect!(
            "E_AGENT_TIMEOUT_RANGE",
            "agents[sync].timeout_s",
            |v| v["agents"][0]["timeout_s"] = json!(seconds)
        );
    }
    expect!("E_AGENT_SKILL", "agents[sync].skills[0]", |v| v["agents"]
        [0]["skills"] =
        json!(["Price_Answers"]));
    expect!("E_AGENT_SKILL", "agents[sync].skills[1]", |v| v["agents"]
        [0]["skills"] =
        json!(["money", "money"]));
    // an agent brings in agents of its plugin and of its dependencies, never itself, each once
    for (delegates, path) in [
        (json!(["sync"]), "agents[sync].delegates[0]"),
        (json!(["ghost"]), "agents[sync].delegates[0]"),
        (json!(["crm.scout"]), "agents[sync].delegates[0]"),
        (json!(["customers."]), "agents[sync].delegates[0]"),
        (
            json!(["customers.scout", "customers.scout"]),
            "agents[sync].delegates[1]",
        ),
    ] {
        let errs = codes_after(|v| v["agents"][0]["delegates"] = delegates.clone());
        assert!(
            has(&errs, "E_AGENT_DELEGATE", path),
            "{delegates}: {errs:?}"
        );
    }
    // the topic an agent's runs invalidate is one the plugin declares
    expect!(
        "E_AGENT_TOPIC_UNKNOWN",
        "agents[sync].topic",
        |v| v["agents"][0]["topic"] = json!("sync_runs")
    );
}

#[test]
fn connections_are_declared_and_named_by_agents() {
    let mut v = orders();
    v["connections"] = json!([
        { "name": "research", "kind": "llm", "title": "Research model" },
        { "name": "speech", "kind": "stt" },
    ]);
    v["agents"][0]["connection"] = json!("research");
    v["agents"][0]["ssh_key"] = json!(true);
    let d: Descriptor = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(validate(&d), vec![]);
    // the connections, the agent's connection and its key are in the manifest: whoever installs the plugin sees them
    let m = to_manifest(&d);
    assert_eq!(m.connections.len(), 2);
    assert_eq!(
        (m.agents[0].connection.as_str(), m.agents[0].ssh_key),
        ("research", true)
    );
    assert!(m.validate().is_ok());
    // a dependency's connection is named <dependency>.<name>
    v["agents"][0]["connection"] = json!("customers.models");
    let d: Descriptor = serde_json::from_value(v).unwrap();
    assert_eq!(validate(&d), vec![]);

    let with = |f: &dyn Fn(&mut Value)| {
        codes_after(|v| {
            v["connections"] =
                json!([{ "name": "research", "kind": "llm" }, { "name": "speech", "kind": "stt" }]);
            f(v)
        })
    };
    for (connection, why) in [
        ("ghost", "not a connection"),
        ("speech", "not llm"),
        ("crm.models", "no dependency"),
        ("customers.Models", "bad shape"),
    ] {
        let errs = with(&|v| v["agents"][0]["connection"] = json!(connection));
        assert!(
            has(&errs, "E_AGENT_CONNECTION", "agents[sync].connection"),
            "{why}: {errs:?}"
        );
    }
    let errs = with(&|v| v["connections"][0]["name"] = json!("Research"));
    assert!(
        has(&errs, "E_CONNECTION_NAME", "connections[Research].name"),
        "{errs:?}"
    );
    let errs = with(&|v| v["connections"][1]["name"] = json!("research"));
    assert!(
        has(&errs, "E_CONNECTION_DUP", "connections[research]"),
        "{errs:?}"
    );
    let errs = with(&|v| v["connections"][1]["kind"] = json!("tts"));
    assert!(
        has(&errs, "E_CONNECTION_KIND", "connections[speech].kind"),
        "{errs:?}"
    );
    let errs = with(&|v| v["connections"][0]["title"] = json!("x".repeat(81)));
    assert!(
        has(&errs, "E_CONNECTION_TEXT", "connections[research]"),
        "{errs:?}"
    );
}

#[test]
fn an_agent_s_delegates_are_in_the_manifest() {
    let mut v = orders();
    v["agents"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "packer" }));
    v["agents"][0]["delegates"] = json!(["packer", "customers.scout"]);
    let d: Descriptor = serde_json::from_value(v).unwrap();
    assert_eq!(validate(&d), vec![]);
    // whom an agent may bring in is what it may do: whoever installs the plugin reviews it with the manifest
    let m = to_manifest(&d);
    assert_eq!(m.agents[0].delegates, ["packer", "customers.scout"]);
    assert!(m.validate().is_ok());
    let mut without = d.clone();
    without.agents[0].delegates.clear();
    assert_ne!(m.digest(), to_manifest(&without).digest());
}

#[test]
fn an_agent_names_a_topic_of_its_plugin() {
    let mut v = orders();
    v["messages"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "AllRuns", "fields": [] }));
    v["topics"] = json!([{ "name": "sync_runs", "key": "AllRuns" }]);
    v["agents"][0]["topic"] = json!("sync_runs");
    let d: Descriptor = serde_json::from_value(v).unwrap();
    assert_eq!(validate(&d), vec![]);
    assert_eq!(d.agents[0].topic, "sync_runs");
    // the topic is how the agent runs, not what it may do: the manifest does not carry it
    let mut without = d.clone();
    without.agents[0].topic.clear();
    assert_eq!(to_manifest(&d), to_manifest(&without));
    // an agent that names none describes itself as before
    let bare = serde_json::to_value(&without).unwrap();
    assert!(bare["agents"][0].get("topic").is_none());
}

#[test]
fn an_agent_says_how_it_runs_and_the_manifest_stays_as_it_was() {
    let bare = orders();
    let mut v = orders();
    v["agents"][0] = json!({
        "name": "sync", "roles": ["clerk"], "title": "Sync", "description": "Keeps orders in step.",
        "engine": "codex", "model": "gpt-5.5", "session": true, "timeout_s": 900, "skills": ["money"],
    });
    let checked = check(v.to_string().as_bytes()).unwrap();
    let a = &checked.descriptor.agents[0];
    assert_eq!(
        (a.engine.as_str(), a.model.as_str(), a.session, a.timeout_s),
        ("codex", "gpt-5.5", true, Some(900))
    );
    assert_eq!(a.skills, ["money"]);
    // none of it is in the manifest: what the plugin may do is what it was, and so is the digest
    let before = to_manifest(&check(bare.to_string().as_bytes()).unwrap().descriptor);
    let after = to_manifest(&checked.descriptor);
    assert_eq!(before, after);
    assert_eq!(before.digest(), after.digest());
    // an agent that declares none of it is written as before
    let plain: Descriptor = serde_json::from_value(bare).unwrap();
    assert_eq!(
        serde_json::to_value(&plain.agents[0]).unwrap(),
        json!({ "name": "sync", "roles": ["clerk"] })
    );
}

#[test]
fn dependencies() {
    expect!(
        "E_DEP_SELF",
        "dependencies[orders].plugin",
        |v| v["dependencies"][0]["plugin"] = json!("orders")
    );
    expect!("E_DEP_DUP", "dependencies[customers]", |v| {
        v["dependencies"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "plugin": "customers" }))
    });
    expect!("E_PLUGIN_ID", "dependencies[1].plugin", |v| {
        v["dependencies"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "plugin": "Bad Name" }))
    });
    expect!("E_ENDPOINT_NAME", "dependencies[customers].calls[0]", |v| {
        v["dependencies"][0]["calls"][0] = json!("get_customer")
    });
}

#[test]
fn messages_enums_fields() {
    expect!("E_MSG_NAME", "messages[line].name", |v| v["messages"][1]
        ["name"] =
        json!("line"));
    expect!(
        "E_MSG_DUP",
        "messages[PlaceOrderRequest]",
        |v| v["messages"][1]["name"] = json!("PlaceOrderRequest")
    );
    expect!("E_MSG_DUP", "messages[OrderStatus]", |v| v["messages"][1]
        ["name"] =
        json!("OrderStatus"));
    expect!("E_MSG_DUP", "entities[order].message", |v| v["entities"]
        [0]["message"] =
        json!("Line"));
    expect!("E_ENUM_DUP", "enums[OrderStatus]", |v| v["enums"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "OrderStatus" })));
    expect!("E_ENUM_DUP", "enums[OrderStatus].values[1]", |v| v["enums"]
        [0]["values"][1] =
        json!("placed"));
    expect!(
        "E_FIELD_NAME",
        "messages[Line].fields[Sku].name",
        |v| v["messages"][1]["fields"][0]["name"] = json!("Sku")
    );
    expect!(
        "E_FIELD_DUP",
        "messages[Line].fields[sku]",
        |v| v["messages"][1]["fields"][1]["name"] = json!("sku")
    );
    expect!(
        "E_FIELD_DUP",
        "messages[Line].fields[quantity].number",
        |v| v["messages"][1]["fields"][1]["number"] = json!(1)
    );
    expect!(
        "E_FIELD_TYPE",
        "messages[Line].fields[sku].type",
        |v| v["messages"][1]["fields"][0]["type"] = json!("text")
    );
    expect!(
        "E_FIELD_TYPE",
        "messages[Line].fields[sku].type",
        |v| v["messages"][1]["fields"][0]["type"] = json!("message:Nope")
    );
    expect!(
        "E_FIELD_TYPE",
        "messages[Line].fields[sku].type",
        |v| v["messages"][1]["fields"][0]["type"] = json!("enum:Nope")
    );
    expect!(
        "E_FIELD_TYPE",
        "messages[Line].fields[sku].type",
        |v| v["messages"][1]["fields"][0]["type"] = json!("ref:orders.invoice")
    );
    expect!(
        "E_FIELD_TYPE",
        "messages[Line].fields[sku].type",
        |v| v["messages"][1]["fields"][0]["type"] = json!("ref:platform.tenant")
    );
    expect!(
        "E_FIELD_TYPE",
        "messages[Line].fields[sku].type",
        |v| v["messages"][1]["fields"][0]["type"] = json!("ref:billing.invoice")
    );
    expect!("E_FIELD_TYPE", "entities[order].fields[status].type", |v| {
        v["entities"][0]["fields"][1]["type"] = json!("enum:")
    });
}

#[test]
fn entities() {
    expect!("E_ENT_NAME", "entities[Order].name", |v| v["entities"][0]
        ["name"] =
        json!("Order"));
    expect!("E_ENT_DUP", "entities[order]", |v| {
        let mut e = v["entities"][0].clone();
        e["message"] = json!("Order2");
        v["entities"].as_array_mut().unwrap().push(e);
    });
    expect!(
        "E_ENT_NAME_RESERVED",
        "entities[order_with_deleted].name",
        |v| v["entities"][0]["name"] = json!("order_with_deleted")
    );
    expect!(
        "E_ENT_KEY",
        "entities[order].key",
        |v| v["entities"][0]["key"] = json!("serial")
    );
}

#[test]
fn endpoints() {
    expect!(
        "E_ENDPOINT_NAME",
        "endpoints[place_order].name",
        |v| v["endpoints"][0]["name"] = json!("place_order")
    );
    expect!(
        "E_ENDPOINT_DUP",
        "endpoints[PlaceOrder]",
        |v| v["endpoints"][1]["name"] = json!("PlaceOrder")
    );
    expect!("E_ENDPOINT_MESSAGE", "endpoints[PlaceOrder].input", |v| v
        ["endpoints"][0]["input"] =
        json!("Nope"));
    expect!("E_ENDPOINT_MESSAGE", "endpoints[PlaceOrder].output", |v| {
        v["endpoints"][0]["output"] = json!("OrderStatus")
    });
    expect!(
        "E_ENDPOINT_ROLE_UNKNOWN",
        "endpoints[PlaceOrder].roles[1]",
        |v| v["endpoints"][0]["roles"][1] = json!("boss")
    );
    expect!(
        "E_ENDPOINT_SCOPE_UNKNOWN",
        "endpoints[PlaceOrder].scopes[0]",
        |v| v["endpoints"][0]["scopes"][0] = json!("admin")
    );
    expect!(
        "E_ENDPOINT_SCOPE_MANY",
        "endpoints[PlaceOrder].scopes",
        |v| v["endpoints"][0]["scopes"] = json!(["read", "write"])
    );
    expect!(
        "E_ENDPOINT_TIMEOUT_RANGE",
        "endpoints[PlaceOrder].timeout_ms",
        |v| v["endpoints"][0]["timeout_ms"] = json!(0)
    );
    expect!(
        "E_ENDPOINT_TIMEOUT_RANGE",
        "endpoints[PlaceOrder].timeout_ms",
        |v| v["endpoints"][0]["timeout_ms"] = json!(600_001)
    );
    // workspace roles need no declaration
    assert!(codes_after(|v| v["endpoints"][0]["roles"][1] = json!("workspace.admin")).is_empty());
}

#[test]
fn all_errors_are_collected() {
    let errs = codes_after(|v| {
        v["id"] = json!("core");
        v["version"] = json!("x");
        v["endpoints"][0]["roles"][0] = json!("ghost");
        v["capabilities"][0]["reason"] = json!("");
    });
    let codes: Vec<&str> = errs.iter().map(|e| e.0).collect();
    assert_eq!(
        codes,
        // the refs to its own entity name the old id: E_FIELD_TYPE as well, and the id is not its own any more
        vec![
            "E_PLUGIN_ID_RESERVED",
            "E_PLUGIN_VERSION",
            "E_CAPABILITY_REASON",
            "E_FIELD_TYPE",
            "E_FIELD_TYPE",
            "E_PK_TYPE",
            "E_ENDPOINT_ROLE_UNKNOWN"
        ]
    );
}

#[test]
fn display_has_code_path_and_help() {
    let e = Error::new(
        "E_ENDPOINT_ROLE_UNKNOWN",
        "endpoints[PlaceOrder].roles[1]",
        "role \"boss\" is not declared",
    )
    .help("declare it");
    assert_eq!(
        e.to_string(),
        "error[E_ENDPOINT_ROLE_UNKNOWN] at endpoints[PlaceOrder].roles[1]: role \"boss\" is not declared\n  help: declare it"
    );
}

#[test]
fn canonical_json_sorts_keys_and_keeps_arrays() {
    let v = json!({ "b": [3, 1, { "z": 1, "a": "x\"y" }], "a": null, "é": 1.5, "A": true });
    assert_eq!(
        canonical_json(&v),
        r#"{"A":true,"a":null,"b":[3,1,{"a":"x\"y","z":1}],"é":1.5}"#
    );
    // the same document in another key order and spacing
    let other =
        r#"{ "A": true, "é": 1.5, "b": [3, 1, {"z": 1, "a": "x\"y"}], "a": null }"#.as_bytes();
    assert_eq!(canonicalize(other).unwrap(), canonical_json(&v));
    let c = check(orders().to_string().as_bytes()).unwrap();
    assert_eq!(
        c.canonical,
        canonical_json(&serde_json::from_str(&c.canonical).unwrap())
    );
}

#[test]
fn manifest_from_descriptor() {
    let c = check(orders().to_string().as_bytes()).unwrap();
    let m = to_manifest(&c.descriptor);
    m.validate().unwrap();
    // read is declared, write is added with the default title
    let scopes: Vec<(&str, &str)> = m
        .scopes
        .iter()
        .map(|s| (s.name.as_str(), s.title.as_str()))
        .collect();
    assert_eq!(
        scopes,
        vec![("write", "Change Orders"), ("read", "Read orders")]
    );
    assert_eq!(m.roles[0].default_for.as_deref(), Some("member"));
    assert_eq!(m.depends, vec!["customers"]);
    assert_eq!(m.calls, vec!["/customers.v1.Customers/GetCustomer"]);
    assert_eq!(m.permissions, vec!["members.email"]);
    assert_eq!(m.agents[0].roles, vec!["clerk"]);
    assert_eq!(m.requires[0].method, "/orders.v1.Orders/PlaceOrder");
    assert_eq!(m.requires[0].scope, "write");
    assert_eq!(m.requires[0].any_role, vec!["clerk", "manager"]);
    assert_eq!(m.requires[1].scope, "read");
    assert!(m.requires[1].any_role.is_empty());
}

#[test]
fn manifest_defaults() {
    let d = parse(
        br#"{"schema":1,"id":"hello","version":"0.1.0",
        "messages":[{"name":"HelloRequest"},{"name":"HelloReply"}],
        "endpoints":[{"name":"Hello","input":"HelloRequest","output":"HelloReply"}]}"#,
    )
    .unwrap();
    assert!(validate(&d).is_empty());
    let m = to_manifest(&d);
    let names: Vec<&str> = m.scopes.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["read", "write"]);
    assert_eq!(m.scopes[0].title, "Read hello");
    assert_eq!(m.requires[0].method, "/hello.v1.Hello/Hello");
    assert_eq!(m.requires[0].scope, "write");
    m.validate().unwrap();
}

#[test]
fn field_types_parse() {
    assert_eq!(
        FieldType::parse("decimal"),
        Some(FieldType::Scalar("decimal"))
    );
    assert_eq!(
        FieldType::parse("message:Line"),
        Some(FieldType::Message("Line".into()))
    );
    assert_eq!(
        FieldType::parse("ref:platform.user"),
        Some(FieldType::Ref {
            plugin: "platform".into(),
            entity: "user".into()
        })
    );
    assert_eq!(FieldType::parse("ref:platform"), None);
    assert_eq!(FieldType::parse("message:"), None);
    assert_eq!(FieldType::parse("float"), None);
}

/// The descriptor the three SDKs print for `sdk/conformance/hello` (an entity whose message is listed in `messages`
/// too) is a valid one.
#[test]
fn the_conformance_hello_validates() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../sdk/conformance/hello/descriptor.json"
    );
    let d = parse(&std::fs::read(path).expect("the fixture")).expect("parses");
    assert_eq!(validate(&d), vec![]);
}

/// The descriptor the three SDKs print for `sdk/conformance/notify` (notification kinds, the capability `push:send`,
/// `ui` endpoints an ask's buttons call, the screen and overlay its links open) is a valid one.
#[test]
fn the_conformance_notify_validates() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../sdk/conformance/notify/descriptor.json"
    );
    let d = parse(&std::fs::read(path).expect("the fixture")).expect("parses");
    assert_eq!(validate(&d), vec![]);
    let kinds: Vec<_> = d
        .notifications
        .iter()
        .map(|k| (k.name.as_str(), k.tab.as_str(), k.digest))
        .collect();
    assert_eq!(
        kinds,
        [
            ("mention", "for_you", true),
            ("thread_reply", "watching", false)
        ]
    );
}

/// The storage members of an entity: per-field `unique` and `index`, entity `indexes`; all optional, so a
/// descriptor without them reads as before.
#[test]
fn entity_storage_members() {
    let mut v = orders();
    v["entities"][0]["fields"][0]["unique"] = json!(true);
    v["entities"][0]["fields"][2]["index"] = json!(true);
    v["entities"][0]["indexes"] = json!([
        { "columns": [{ "name": "created_by" }, { "name": "created_at", "desc": true }, { "name": "id" }] },
        { "columns": [{ "name": "status" }, { "name": "number" }], "unique": true }
    ]);
    assert_eq!(errors(&v), vec![]);
    let d = parse(v.to_string().as_bytes()).unwrap();
    let e = &d.entities[0];
    assert!(e.fields[0].unique && !e.fields[0].index);
    assert!(e.fields[2].index);
    assert_eq!(e.indexes.len(), 2);
    assert!(e.indexes[0].columns[1].desc && e.indexes[1].unique);
    // absent members are not written back
    let back = serde_json::to_value(parse(orders().to_string().as_bytes()).unwrap()).unwrap();
    assert!(back["entities"][0].get("indexes").is_none());
    assert!(back["entities"][0]["fields"][0].get("unique").is_none());
    assert!(back["entities"][0]["fields"][0].get("index").is_none());
}

#[test]
fn entity_rules() {
    expect!("E_ENT_LABEL", "entities[order].label", |v| v["entities"]
        [0]["label"] =
        json!("nope"));
    expect!("E_ENT_APPEND_ONLY_VERSION", "entities[order]", |v| {
        v["entities"][0]["append_only"] = json!(true)
    });
    expect!("E_ENT_SOFT_DELETE_APPEND_ONLY", "entities[order]", |v| {
        v["entities"][0]["version"] = json!(false);
        v["entities"][0]["append_only"] = json!(true);
        v["entities"][0]["soft_delete"] = json!(true);
    });
    expect!("E_PK_MISSING", "entities[order]", |v| {
        v["entities"][0]["fields"].as_array_mut().unwrap().remove(5);
    });
    expect!("E_PK_TYPE", "entities[order].fields[id].type", |v| {
        v["entities"][0]["fields"][5]["type"] = json!("int64")
    });
    expect!("E_PK_TYPE", "entities[order].fields[id].type", |v| {
        v["entities"][0]["fields"][5]["optional"] = json!(true)
    });
    expect!(
        "E_COL_RESERVED",
        "entities[order].fields[created_at].name",
        |v| {
            let f = json!({ "name": "created_at", "number": 9, "type": "timestamp", "repeated": false, "optional": false, "description": "" });
            v["entities"][0]["fields"].as_array_mut().unwrap().push(f);
        }
    );
    expect!("E_COL_MODIFIER", "messages[Line].fields[sku]", |v| {
        v["messages"][1]["fields"][0]["unique"] = json!(true)
    });
    expect!("E_UNIQUE_TYPE", "entities[order].fields[total]", |v| {
        v["entities"][0]["fields"][4]["type"] = json!("json");
        v["entities"][0]["fields"][4]["unique"] = json!(true);
    });
    expect!(
        "E_IDX_COLUMN",
        "entities[order].indexes[0].columns[0]",
        |v| { v["entities"][0]["indexes"] = json!([{ "columns": [{ "name": "nope" }] }]) }
    );
    // deleted_at exists only on a soft-delete entity
    expect!(
        "E_IDX_COLUMN",
        "entities[order].indexes[0].columns[0]",
        |v| { v["entities"][0]["indexes"] = json!([{ "columns": [{ "name": "deleted_at" }] }]) }
    );
    expect!("E_IDX_COLUMN", "entities[order].indexes[0]", |v| {
        v["entities"][0]["indexes"] = json!([{ "columns": [] }])
    });
    expect!(
        "E_IDX_COLUMN",
        "entities[order].indexes[0].columns[1]",
        |v| {
            v["entities"][0]["indexes"] =
                json!([{ "columns": [{ "name": "status" }, { "name": "status" }] }])
        }
    );
    // the name `<plugin>_<entity>` must stay within 50 characters
    let long = "a".repeat(44);
    let errs = codes_after(|v| {
        v["entities"][0]["name"] = json!(long);
        v["entities"][0]["fields"][3]["type"] = json!(format!("ref:orders.{long}"));
        v["entities"][0]["fields"][5]["type"] = json!(format!("ref:orders.{long}"));
    });
    assert_eq!(
        errs,
        vec![("E_ENT_NAME_LENGTH", format!("entities[{long}].name"))]
    );
    // system columns may be named
    let errs = codes_after(|v| {
        v["entities"][0]["indexes"] =
            json!([{ "columns": [{ "name": "version" }, { "name": "updated_at" }] }])
    });
    assert_eq!(errs, vec![]);
}

/// An entity's message listed in `messages` too (as the SDKs print it) is the same message, not a duplicate; a
/// different message of that name is.
#[test]
fn an_entity_message_listed_in_messages() {
    let mut v = orders();
    let fields = v["entities"][0]["fields"].clone();
    v["messages"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "Order", "kind": "message", "description": "", "fields": fields }));
    assert_eq!(errors(&v), vec![]);
    v["messages"][2]["fields"].as_array_mut().unwrap().pop();
    assert!(has(
        &errors(&v)
            .into_iter()
            .map(|e| (e.code, e.path))
            .collect::<Vec<_>>(),
        "E_MSG_DUP",
        "entities[order].message"
    ));
}

/// `orders` with an optional encrypted field `note` on `order`, then changed by `f`.
fn encrypted_after(f: impl FnOnce(&mut Value)) -> Vec<(&'static str, String)> {
    codes_after(|v| {
        v["entities"][0]["fields"].as_array_mut().unwrap().push(json!(
            { "name": "note", "number": 7, "type": "string", "optional": true, "encrypted": true }
        ));
        f(v)
    })
}

#[test]
fn an_encrypted_field_is_only_a_value() {
    assert_eq!(encrypted_after(|_| {}), vec![]);
    let field = |v: &mut Value, name: &str, k: &str, x: Value| {
        let fields = v["entities"][0]["fields"].as_array_mut().unwrap();
        let f = fields.iter_mut().find(|f| f["name"] == name).unwrap();
        f[k] = x;
    };
    let has_code = |errs: &[(&'static str, String)], code: &str, path: &str| {
        assert!(
            has(errs, code, path),
            "expected {code} at {path}, got {errs:?}"
        );
    };
    let e = encrypted_after(|v| field(v, "id", "encrypted", json!(true)));
    has_code(&e, "E_ENCRYPTED_KEY", "entities[order].fields[id]");
    let e = encrypted_after(|v| field(v, "number", "encrypted", json!(true)));
    has_code(&e, "E_ENCRYPTED_LABEL", "entities[order].fields[number]");
    let e = encrypted_after(|v| field(v, "created_by", "encrypted", json!(true)));
    has_code(&e, "E_ENCRYPTED_REF", "entities[order].fields[created_by]");
    let e = encrypted_after(|v| field(v, "note", "unique", json!(true)));
    has_code(&e, "E_ENCRYPTED_INDEX", "entities[order].fields[note]");
    let e = encrypted_after(|v| field(v, "note", "index", json!(true)));
    has_code(&e, "E_ENCRYPTED_INDEX", "entities[order].fields[note]");
    // a searchable column keeps its text in its search indexes
    let e = encrypted_after(|v| field(v, "note", "searchable", json!("english")));
    has_code(&e, "E_ENCRYPTED_INDEX", "entities[order].fields[note]");
    let e = encrypted_after(|v| {
        v["entities"][0]["indexes"] =
            json!([{ "columns": [{ "name": "total" }, { "name": "note" }] }]);
    });
    has_code(
        &e,
        "E_ENCRYPTED_INDEX",
        "entities[order].indexes[0].columns[1]",
    );
    // a message's field is no column: encrypted is a column modifier
    let e = encrypted_after(|v| v["messages"][1]["fields"][0]["encrypted"] = json!(true));
    has_code(&e, "E_COL_MODIFIER", "messages[Line].fields[sku]");
    // the rules never read an encrypted column
    let e = encrypted_after(|v| {
        v["entities"][0]["access"] = json!({
            "relations": [{ "name": "author", "kind": "column", "field": "note" }],
            "read": { "when": { "grant": "authenticated", "filter": [{ "field": "note", "op": "is_null", "value": null }] } },
            "update": { "role": "clerk" },
            "guards": [{ "fields": ["note"], "rules": [{ "value": null, "grant": { "role": "manager" } }] }],
            "masks": [{ "field": "note", "roles": ["manager"] }]
        });
    });
    has_code(
        &e,
        "E_ENCRYPTED_REF",
        "entities[order].access.relations[0].field",
    );
    has_code(&e, "E_ENCRYPTED_WHEN", "entities[order].access.read");
    has_code(
        &e,
        "E_ENCRYPTED_GUARD",
        "entities[order].access.guards[0].fields[0]",
    );
    // a mask by role is fine
    assert!(
        !e.iter()
            .any(|(c, p)| p.contains("masks") && c.starts_with("E_ENCRYPTED")),
        "{e:?}"
    );
    // printed only when set, and read back
    let d = parse(orders().to_string().as_bytes()).unwrap();
    assert!(!serde_json::to_string(&d).unwrap().contains("encrypted"));
}

#[test]
fn an_agent_asking_for_admin_reaches_the_manifest_and_its_digest() {
    let mut v = orders();
    let plain = to_manifest(&serde_json::from_value::<Descriptor>(v.clone()).unwrap());
    v["agents"][0]["admin"] = json!(true);
    let d: Descriptor = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(validate(&d), vec![]);
    let m = to_manifest(&d);
    assert!(m.agents[0].admin && m.to_pb().agents[0].admin);
    assert_ne!(m.digest(), plain.digest());
    // the descriptor writes it back as declared, and leaves it out when not
    assert_eq!(
        serde_json::to_value(&d).unwrap()["agents"][0]["admin"],
        json!(true)
    );
    v["agents"][0]["admin"] = json!(false);
    let d: Descriptor = serde_json::from_value(v.clone()).unwrap();
    assert!(
        serde_json::to_value(&d).unwrap()["agents"][0]
            .get("admin")
            .is_none()
    );
    assert_eq!(to_manifest(&d).digest(), plain.digest());
    // a base role is admin or nothing
    v["agents"][0]["admin"] = json!("owner");
    assert!(serde_json::from_value::<Descriptor>(v).is_err());
}

/// `searchable`: a single string column of an entity, in a known language, at most eight per entity; written only
/// when declared.
#[test]
fn searchable_columns() {
    let mut v = orders();
    v["entities"][0]["fields"][0]["searchable"] = json!("simple");
    assert_eq!(errors(&v), vec![]);
    let d = parse(v.to_string().as_bytes()).unwrap();
    assert_eq!(d.entities[0].fields[0].searchable, "simple");
    let back = serde_json::to_value(parse(orders().to_string().as_bytes()).unwrap()).unwrap();
    assert!(back["entities"][0]["fields"][0].get("searchable").is_none());

    expect!("E_SEARCH_TYPE", "entities[order].fields[total]", |v| {
        v["entities"][0]["fields"][4]["searchable"] = json!("simple")
    });
    expect!("E_SEARCH_TYPE", "entities[order].fields[number]", |v| {
        v["entities"][0]["fields"][0]["repeated"] = json!(true);
        v["entities"][0]["fields"][0]["searchable"] = json!("english");
    });
    expect!(
        "E_SEARCH_LANGUAGE",
        "entities[order].fields[number].searchable",
        |v| { v["entities"][0]["fields"][0]["searchable"] = json!("klingon") }
    );
    expect!("E_COL_MODIFIER", "messages[Line].fields[sku]", |v| {
        v["messages"][1]["fields"][0]["searchable"] = json!("simple")
    });
    // nine searchable columns: the ninth is refused
    let errs = codes_after(|v| {
        let fields = v["entities"][0]["fields"].as_array_mut().unwrap();
        for i in 0..9 {
            fields.push(json!({ "name": format!("text_{i}"), "number": 10 + i, "type": "string",
                                "repeated": false, "optional": true, "description": "", "searchable": "english" }));
        }
    });
    assert_eq!(
        errs,
        vec![("E_SEARCH_LIMIT", "entities[order].fields[text_8]".into())]
    );
}

/// Navigations: a ref's forward nav (declared, or its name without `_id`) and an inverse nav onto an entity of the
/// plugin; names are snake_case and unique on their entity.
#[test]
fn navigations() {
    let line = |v: &mut Value, order_id: Value| {
        v["entities"].as_array_mut().unwrap().push(json!({
            "name": "line", "message": "Line2", "key": "i64",
            "fields": [
                { "name": "id", "number": 1, "type": "ref:orders.line", "repeated": false, "optional": false, "description": "" },
                order_id,
                { "name": "customer_id", "number": 3, "type": "ref:customers.customer", "repeated": false, "optional": true, "description": "" }
            ] }));
    };
    let order_id = |extra: Value| {
        let mut f = json!({ "name": "order_id", "number": 2, "type": "ref:orders.order", "repeated": false, "optional": false, "description": "" });
        for (k, x) in extra.as_object().unwrap() {
            f[k] = x.clone();
        }
        f
    };
    let mut v = orders();
    line(
        &mut v,
        order_id(json!({ "nav": "order", "inverse": "lines" })),
    );
    assert_eq!(errors(&v), vec![]);
    let d = parse(v.to_string().as_bytes()).unwrap();
    let mut navs = navs(&d, "order");
    navs.sort_by(|a, b| a.0.cmp(&b.0));
    let names: Vec<&str> = navs.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["created_by", "lines", "parent"]);
    assert_eq!(
        navs[1].1,
        Nav::Inverse {
            entity: "line".into(),
            column: "order_id".into()
        }
    );
    let line_navs = super::navs(&d, "line");
    let names: Vec<&str> = line_navs.iter().map(|(n, _)| n.as_str()).collect();
    // a forward nav by its name and by its ref column's
    assert_eq!(names, ["order_id", "order", "customer_id", "customer"]);
    assert_eq!(
        line_navs[3].1,
        Nav::Forward {
            column: "customer_id".into(),
            plugin: "customers".into(),
            entity: "customer".into()
        }
    );

    let with = |f: Value| {
        let mut v = orders();
        line(&mut v, f);
        errors(&v)
            .into_iter()
            .map(|e| (e.code, e.path))
            .collect::<Vec<_>>()
    };
    let at = "entities[line].fields[order_id]";
    assert!(has(
        &with(order_id(json!({ "nav": "Order" }))),
        "E_REF_NAME",
        &format!("{at}.nav")
    ));
    // a nav named like another field
    assert!(has(
        &with(order_id(json!({ "nav": "customer_id" }))),
        "E_REF_NAV_DUP",
        &format!("{at}.nav")
    ));
    // two refs with the same (default) nav
    assert!(has(
        &with(order_id(json!({ "nav": "customer" }))),
        "E_REF_NAV_DUP",
        "entities[line].fields[customer_id].nav"
    ));
    // an inverse named like a field of the target, or like its forward nav
    assert!(has(
        &with(order_id(json!({ "inverse": "total" }))),
        "E_REF_INVERSE_DUP",
        &format!("{at}.inverse")
    ));
    assert!(has(
        &with(order_id(json!({ "inverse": "parent" }))),
        "E_REF_INVERSE_DUP",
        &format!("{at}.inverse")
    ));
    // an inverse onto another plugin's entity
    let mut v = orders();
    line(&mut v, order_id(json!({})));
    v["entities"][1]["fields"][2]["inverse"] = json!("lines");
    assert!(has(
        &errors(&v)
            .into_iter()
            .map(|e| (e.code, e.path))
            .collect::<Vec<_>>(),
        "E_REF_INVERSE_TARGET",
        "entities[line].fields[customer_id].inverse"
    ));
    // two inverses of one name onto the same entity
    let mut v = orders();
    line(&mut v, order_id(json!({ "inverse": "lines" })));
    v["entities"][0]["fields"][3]["inverse"] = json!("lines");
    let errs: Vec<_> = errors(&v).into_iter().map(|e| (e.code, e.path)).collect();
    assert!(
        has(&errs, "E_REF_INVERSE_DUP", &format!("{at}.inverse")),
        "{errs:?}"
    );
    // nav on a field that is not a ref
    expect!("E_REF_NAME", "entities[order].fields[total].nav", |v| {
        v["entities"][0]["fields"][4]["nav"] = json!("money")
    });
}

/// `on_delete`: within the plugin `restrict` (default), `cascade`, `set_null` (an optional ref), `no_action`; across
/// plugins `keep` (default), and `set_null` onto the directory only; no `set_null` in an append-only entity.
#[test]
fn on_delete_of_refs() {
    let line = |v: &mut Value, append_only: bool, order_id: Value, customer_id: Value| {
        let mut order = json!({ "name": "order_id", "number": 2, "type": "ref:orders.order", "repeated": false, "optional": true, "description": "" });
        let mut customer = json!({ "name": "customer_id", "number": 3, "type": "ref:customers.customer", "repeated": false, "optional": true, "description": "" });
        for (f, extra) in [(&mut order, order_id), (&mut customer, customer_id)] {
            for (k, x) in extra.as_object().unwrap() {
                f[k] = x.clone();
            }
        }
        v["entities"].as_array_mut().unwrap().push(json!({
            "name": "line", "message": "Line2", "key": "i64", "append_only": append_only,
            "fields": [
                { "name": "id", "number": 1, "type": "ref:orders.line", "repeated": false, "optional": false, "description": "" },
                order,
                customer,
                { "name": "team_id", "number": 4, "type": "ref:platform.group", "repeated": false, "optional": true, "description": "", "on_delete": "set_null" }
            ] }));
    };
    let with = |append_only: bool, o: Value, c: Value| {
        let mut v = orders();
        line(&mut v, append_only, o, c);
        errors(&v)
            .into_iter()
            .map(|e| (e.code, e.path))
            .collect::<Vec<_>>()
    };
    for value in ["restrict", "cascade", "set_null", "no_action"] {
        assert_eq!(
            with(false, json!({ "on_delete": value }), json!({})),
            vec![],
            "{value}"
        );
    }
    assert_eq!(
        with(false, json!({}), json!({ "on_delete": "keep" })),
        vec![]
    );
    let order_at = "entities[line].fields[order_id].on_delete";
    let customer_at = "entities[line].fields[customer_id].on_delete";
    assert_eq!(
        with(false, json!({ "on_delete": "keep" }), json!({})),
        vec![("E_REF_ON_DELETE", order_at.into())]
    );
    assert_eq!(
        with(false, json!({ "on_delete": "erase" }), json!({})),
        vec![("E_REF_ON_DELETE", order_at.into())]
    );
    // set_null needs an optional ref
    assert_eq!(
        with(
            false,
            json!({ "on_delete": "set_null", "optional": false }),
            json!({})
        ),
        vec![("E_REF_ON_DELETE", order_at.into())]
    );
    // across plugins: keep only (the directory also set_null)
    for value in ["cascade", "set_null", "restrict"] {
        assert_eq!(
            with(false, json!({}), json!({ "on_delete": value })),
            vec![("E_REF_CROSS_ON_DELETE", customer_at.into())],
            "{value}"
        );
    }
    // an append-only entity's rows never change: no set_null (the directory's included), cascade is how they go
    assert_eq!(
        with(true, json!({ "on_delete": "set_null" }), json!({})),
        vec![
            ("E_ENT_APPEND_ONLY_CASCADE", order_at.into()),
            (
                "E_ENT_APPEND_ONLY_CASCADE",
                "entities[line].fields[team_id].on_delete".into()
            )
        ]
    );
    let errs = with(true, json!({ "on_delete": "cascade" }), json!({}));
    assert_eq!(
        errs,
        vec![(
            "E_ENT_APPEND_ONLY_CASCADE",
            "entities[line].fields[team_id].on_delete".into()
        )]
    );
    // not a ref; a message's field
    expect!(
        "E_REF_ON_DELETE",
        "entities[order].fields[total].on_delete",
        |v| { v["entities"][0]["fields"][4]["on_delete"] = json!("cascade") }
    );
    expect!("E_COL_MODIFIER", "messages[Line].fields[sku]", |v| {
        v["messages"][1]["fields"][0]["on_delete"] = json!("cascade")
    });
    // the default, and the descriptor keeps only what was declared
    let mut v = orders();
    line(&mut v, false, json!({}), json!({}));
    let d = parse(v.to_string().as_bytes()).unwrap();
    let fields = &d.entities.last().unwrap().fields;
    assert_eq!(
        fields[1].on_delete_or_default("orders").as_deref(),
        Some("restrict")
    );
    assert_eq!(
        fields[2].on_delete_or_default("orders").as_deref(),
        Some("keep")
    );
    assert_eq!(
        fields[3].on_delete_or_default("orders").as_deref(),
        Some("set_null")
    );
    assert_eq!(fields[0].on_delete_or_default("orders"), None);
    let back = serde_json::to_value(&d).unwrap();
    assert!(back["entities"][1]["fields"][1].get("on_delete").is_none());
}

/// The example with two jobs: a scheduled one and one only enqueued.
fn with_jobs() -> Value {
    let mut v = orders();
    v["messages"].as_array_mut().unwrap().extend([
        json!({ "name": "RemindUnpaid", "kind": "job",
                "fields": [{ "name": "older_than_days", "number": 1, "type": "int32" }] }),
        json!({ "name": "PostReport", "kind": "job",
                "fields": [{ "name": "day", "number": 1, "type": "date" },
                           { "name": "line", "number": 2, "type": "message:Line" }] }),
    ]);
    v["jobs"] = json!([
        { "name": "remind_unpaid", "payload": "RemindUnpaid", "description": "job.remind_unpaid.description",
          "cron": "0 9 * * 1-5", "as_system": true,
          "retry": { "retries": 3, "delay_ms": 60000, "backoff": "exponential" },
          "timeout_ms": 120000, "concurrency": 1 },
        { "name": "post_report", "payload": "PostReport" }
    ]);
    v
}

fn job_codes_after(f: impl FnOnce(&mut Value)) -> Vec<(&'static str, String)> {
    let mut v = with_jobs();
    f(&mut v);
    errors(&v).into_iter().map(|e| (e.code, e.path)).collect()
}

macro_rules! expect_job {
    ($code:literal, $path:literal, $f:expr) => {{
        let errs = job_codes_after($f);
        assert!(
            has(&errs, $code, $path),
            "expected {} at {}, got {:?}",
            $code,
            $path,
            errs
        );
    }};
}

#[test]
fn jobs_are_valid_and_take_defaults() {
    let v = with_jobs();
    assert_eq!(errors(&v), vec![]);
    let d = parse(v.to_string().as_bytes()).unwrap();
    let remind = &d.jobs[0];
    assert!(remind.as_system);
    assert_eq!(remind.schedule().unwrap().unwrap().as_str(), "0 9 * * 1-5");
    let report = &d.jobs[1];
    assert!(report.schedule().is_none());
    assert_eq!(
        report.retry_or_default(),
        JobRetry {
            retries: 3,
            delay_ms: 30_000,
            backoff: Backoff::Exponential
        }
    );
    assert_eq!(report.timeout_ms_or_default(), 300_000);
    assert_eq!(report.concurrency_or_default(), 1);
    // a partial retry takes the defaults of the rest
    let r: JobRetry = serde_json::from_value(json!({ "backoff": "fixed" })).unwrap();
    assert_eq!((r.retries, r.delay_ms), (3, 30_000));
    // only what was declared is written back, and no jobs: no member
    let back = serde_json::to_value(&d).unwrap();
    assert_eq!(
        back["jobs"][1],
        json!({ "name": "post_report", "payload": "PostReport" })
    );
    let plain = parse(orders().to_string().as_bytes()).unwrap();
    assert!(serde_json::to_value(&plain).unwrap().get("jobs").is_none());
    // jobs are not in the manifest
    assert_eq!(to_manifest(&d), to_manifest(&plain));
}

#[test]
fn job_errors() {
    expect_job!("E_JOB_NAME", "jobs[RemindUnpaid].name", |v| {
        v["jobs"][0]["name"] = json!("RemindUnpaid")
    });
    expect_job!(
        "E_JOB_NAME",
        "jobs[aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa].name",
        |v| { v["jobs"][0]["name"] = json!("a".repeat(33)) }
    );
    expect_job!("E_JOB_DUP", "jobs[remind_unpaid]", |v| {
        v["jobs"][1]["name"] = json!("remind_unpaid")
    });
    expect_job!("E_JOB_PAYLOAD", "jobs[post_report].payload", |v| {
        v["jobs"][1]["payload"] = json!("Missing")
    });
    expect_job!("E_JOB_PAYLOAD_DUP", "jobs[post_report].payload", |v| {
        v["jobs"][1]["payload"] = json!("RemindUnpaid")
    });
    expect_job!("E_JOB_CRON", "jobs[remind_unpaid].cron", |v| {
        v["jobs"][0]["cron"] = json!("0 9 * *")
    });
    expect_job!("E_JOB_CRON", "jobs[remind_unpaid].cron", |v| {
        v["jobs"][0]["cron"] = json!("0 25 * * *")
    });
    // a required date the schedule cannot fill: directly, and in a required message
    expect_job!("E_JOB_SCHEDULE_PAYLOAD", "jobs[post_report].payload", |v| {
        v["jobs"][1]["cron"] = json!("0 0 * * *")
    });
    expect_job!(
        "E_JOB_SCHEDULE_PAYLOAD",
        "jobs[remind_unpaid].payload",
        |v| {
            v["messages"][2]["fields"]
                .as_array_mut()
                .unwrap()
                .push(json!({ "name": "line", "number": 2, "type": "message:Line" }));
            v["messages"][1]["fields"][0]["type"] = json!("timestamp");
        }
    );
    assert!(
        job_codes_after(|v| {
            v["jobs"][1]["cron"] = json!("0 0 * * *");
            v["messages"][3]["fields"][0]["optional"] = json!(true);
        })
        .is_empty()
    );
    expect_job!("E_JOB_RETRY", "jobs[remind_unpaid].retry.retries", |v| {
        v["jobs"][0]["retry"]["retries"] = json!(21)
    });
    expect_job!("E_JOB_RETRY", "jobs[remind_unpaid].retry.delay_ms", |v| {
        v["jobs"][0]["retry"]["delay_ms"] = json!(3_600_001)
    });
    expect_job!("E_JOB_TIMEOUT", "jobs[remind_unpaid].timeout_ms", |v| {
        v["jobs"][0]["timeout_ms"] = json!(999)
    });
    expect_job!("E_JOB_TIMEOUT", "jobs[remind_unpaid].timeout_ms", |v| {
        v["jobs"][0]["timeout_ms"] = json!(3_600_001)
    });
    expect_job!(
        "E_JOB_CONCURRENCY",
        "jobs[remind_unpaid].concurrency",
        |v| { v["jobs"][0]["concurrency"] = json!(0) }
    );
    expect_job!(
        "E_JOB_CONCURRENCY",
        "jobs[remind_unpaid].concurrency",
        |v| { v["jobs"][0]["concurrency"] = json!(65) }
    );
    // an unknown backoff is a schema error
    expect_job!("E_SCHEMA", "jobs[0].retry.backoff", |v| {
        v["jobs"][0]["retry"]["backoff"] = json!("linear")
    });
}
