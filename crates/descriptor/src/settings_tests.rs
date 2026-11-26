use serde_json::{Map, Value, json};

use super::*;
use crate::{check, lock};

fn field(name: &str, ty: &str, extra: Value) -> Value {
    let mut f = json!({ "name": name, "number": 1, "type": ty, "repeated": false, "optional": false,
                        "description": "" });
    for (k, v) in extra.as_object().unwrap() {
        f[k] = v.clone();
    }
    f
}

fn input(name: &str, control: &str) -> Value {
    json!({ "name": name, "control": control, "label": name })
}

/// A `chat` plugin with settings: a limit, a URL, a secret, an enum, a list of members.
fn chat() -> Value {
    json!({
      "schema": 1, "id": "chat", "version": "1.0.0",
      "enums": [{ "name": "Tone", "values": ["calm", "loud_and_clear"] }],
      "messages": [
        { "name": "ChatSettings", "kind": "settings", "description": "", "fields": [
            field("thread_warm_hours", "int32", json!({"label": "Thread stays with its agent, hours", "default": 2,
                                                          "min": 1, "max": 48, "section": "Agents"})),
            field("message_max_chars", "int32", json!({"default": 20000, "min": 100, "max": 20000})),
            field("webhook_url", "string", json!({"optional": true, "format": "url", "section": "Integrations"})),
            field("webhook_token", "secret", json!({"optional": true, "max_len": 200, "section": "Integrations"})),
            field("tone", "enum:Tone", json!({"default": "calm"})),
            field("watchers", "ref:platform.user", json!({"repeated": true, "max_items": 3})),
        ] }
      ],
      "settings": { "message": "ChatSettings", "on_change": true, "form": { "sections": [
        { "title": "Agents", "inputs": [input("thread_warm_hours", "number")] },
        { "inputs": [input("message_max_chars", "number"), input("tone", "select"), input("watchers", "member")] },
        { "title": "Integrations", "inputs": [input("webhook_url", "text"), input("webhook_token", "secret")] }
      ] } }
    })
}

/// The descriptor with its fields numbered in order, as an SDK prints them.
fn numbered(v: &Value) -> Value {
    let mut v = v.clone();
    for m in v["messages"].as_array_mut().unwrap() {
        for (i, f) in m["fields"].as_array_mut().unwrap().iter_mut().enumerate() {
            f["number"] = json!(i + 1);
        }
    }
    v
}

fn desc(v: &Value) -> Descriptor {
    crate::parse(numbered(v).to_string().as_bytes()).unwrap()
}

fn codes(v: &Value) -> Vec<(&'static str, String)> {
    match check(numbered(v).to_string().as_bytes()) {
        Ok(_) => vec![],
        Err(e) => e.into_iter().map(|e| (e.code, e.path)).collect(),
    }
}

fn codes_after(f: impl FnOnce(&mut Value)) -> Vec<(&'static str, String)> {
    let mut v = chat();
    f(&mut v);
    codes(&v)
}

fn has(codes: &[(&'static str, String)], code: &str, path: &str) -> bool {
    codes.iter().any(|(c, p)| *c == code && p == path)
}

fn map(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn a_consistent_settings_section_checks() {
    assert_eq!(codes(&chat()), vec![]);
    // the section round-trips
    let d = desc(&chat());
    let back = serde_json::to_value(&d).unwrap();
    assert_eq!(back["settings"]["on_change"], json!(true));
    assert_eq!(
        back["messages"][0]["fields"][0]["label"],
        json!("Thread stays with its agent, hours")
    );
    assert!(back["messages"][0]["fields"][2].get("label").is_none());
}

#[test]
fn the_settings_message_is_a_declared_settings_message() {
    let c = codes_after(|v| v["settings"]["message"] = json!("Nope"));
    assert!(has(&c, "E_SETTINGS_MESSAGE", "settings.message"), "{c:?}");
    let c = codes_after(|v| v["messages"][0]["kind"] = json!("message"));
    assert!(has(&c, "E_SETTINGS_MESSAGE", "settings.message"), "{c:?}");
}

#[test]
fn settings_take_a_few_types() {
    let c = codes_after(|v| v["messages"][0]["fields"][0]["type"] = json!("json"));
    assert!(
        has(
            &c,
            "E_SETTINGS_FIELD_TYPE",
            "messages[ChatSettings].fields[thread_warm_hours].type"
        ),
        "{c:?}"
    );
    // a list of numbers, an optional list, a ref to an entity: no
    let c = codes_after(|v| {
        v["messages"][0]["fields"][1]["repeated"] = json!(true);
        v["messages"][0]["fields"][1]
            .as_object_mut()
            .unwrap()
            .retain(|k, _| !["default", "min", "max"].contains(&k.as_str()));
        v["messages"][0]["fields"][5]["optional"] = json!(true);
    });
    assert!(
        has(
            &c,
            "E_SETTINGS_FIELD_TYPE",
            "messages[ChatSettings].fields[message_max_chars].type"
        ),
        "{c:?}"
    );
    assert!(
        has(
            &c,
            "E_SETTINGS_FIELD_TYPE",
            "messages[ChatSettings].fields[watchers].type"
        ),
        "{c:?}"
    );
    // a secret anywhere else
    let c = codes_after(|v| {
        v["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "name": "Other", "kind": "message",
            "fields": [field("key", "secret", json!({}))] }))
    });
    assert!(
        has(
            &c,
            "E_SETTINGS_FIELD_TYPE",
            "messages[Other].fields[key].type"
        ),
        "{c:?}"
    );
}

#[test]
fn defaults_pass_their_own_validators() {
    let c = codes_after(|v| v["messages"][0]["fields"][0]["default"] = json!(49));
    assert!(
        has(
            &c,
            "E_SETTINGS_DEFAULT",
            "messages[ChatSettings].fields[thread_warm_hours].default"
        ),
        "{c:?}"
    );
    // written as its type is: a number as a number
    let c = codes_after(|v| v["messages"][0]["fields"][0]["default"] = json!("2"));
    assert!(
        has(
            &c,
            "E_SETTINGS_DEFAULT",
            "messages[ChatSettings].fields[thread_warm_hours].default"
        ),
        "{c:?}"
    );
    let c = codes_after(|v| v["messages"][0]["fields"][4]["default"] = json!("shout"));
    assert!(
        has(
            &c,
            "E_SETTINGS_DEFAULT",
            "messages[ChatSettings].fields[tone].default"
        ),
        "{c:?}"
    );
    let c = codes_after(|v| v["messages"][0]["fields"][3]["default"] = json!("sk-1"));
    assert!(
        has(
            &c,
            "E_SETTINGS_DEFAULT",
            "messages[ChatSettings].fields[webhook_token].default"
        ),
        "{c:?}"
    );
}

#[test]
fn validators_fit_their_field() {
    let p = "messages[ChatSettings].fields";
    let c = codes_after(|v| v["messages"][0]["fields"][2]["min"] = json!(1));
    assert!(
        has(&c, "E_SETTINGS_VALIDATOR", &format!("{p}[webhook_url].min")),
        "{c:?}"
    );
    let c = codes_after(|v| v["messages"][0]["fields"][0]["min"] = json!(50));
    assert!(
        has(
            &c,
            "E_SETTINGS_VALIDATOR",
            &format!("{p}[thread_warm_hours].min")
        ),
        "{c:?}"
    );
    let c = codes_after(|v| v["messages"][0]["fields"][0]["max"] = json!("48"));
    assert!(
        has(
            &c,
            "E_SETTINGS_VALIDATOR",
            &format!("{p}[thread_warm_hours].max")
        ),
        "{c:?}"
    );
    let c = codes_after(|v| v["messages"][0]["fields"][0]["max_len"] = json!(5));
    assert!(
        has(
            &c,
            "E_SETTINGS_VALIDATOR",
            &format!("{p}[thread_warm_hours].min_len")
        ),
        "{c:?}"
    );
    let c = codes_after(|v| v["messages"][0]["fields"][3]["pattern"] = json!("^a$"));
    assert!(
        has(
            &c,
            "E_SETTINGS_VALIDATOR",
            &format!("{p}[webhook_token].pattern")
        ),
        "{c:?}"
    );
    let c = codes_after(|v| v["messages"][0]["fields"][2]["format"] = json!("phone"));
    assert!(
        has(
            &c,
            "E_SETTINGS_VALIDATOR",
            &format!("{p}[webhook_url].format")
        ),
        "{c:?}"
    );
    let c = codes_after(|v| v["messages"][0]["fields"][2]["max_items"] = json!(2));
    assert!(
        has(
            &c,
            "E_SETTINGS_VALIDATOR",
            &format!("{p}[webhook_url].min_items")
        ),
        "{c:?}"
    );
    let c = codes_after(|v| v["messages"][0]["fields"][5]["min_items"] = json!(4));
    assert!(
        has(
            &c,
            "E_SETTINGS_VALIDATOR",
            &format!("{p}[watchers].min_items")
        ),
        "{c:?}"
    );
    let c = codes_after(|v| v["messages"][0]["fields"][0]["multiline"] = json!(true));
    assert!(
        has(
            &c,
            "E_SETTINGS_VALIDATOR",
            &format!("{p}[thread_warm_hours].multiline")
        ),
        "{c:?}"
    );
}

#[test]
fn patterns_are_read_the_same_by_both_engines() {
    for ok in [
        "^[A-Z]{3}$",
        r"^\d{2,4}-\w+$",
        r"^(?:ab|cd)+?\.x$",
        r"^[^a-z\-_]*$",
        r"a\/b\(c\)",
        "^.{0,5}$",
        "^x{2,}$",
    ] {
        assert!(
            compile_pattern(ok).is_ok(),
            "{ok}: {:?}",
            compile_pattern(ok).err()
        );
    }
    for bad in [
        r"\s+",
        r"\bword\b",
        r"\p{L}",
        "(?i)abc",
        "(?P<n>a)",
        "(?=a)",
        r"(a)\1",
        "[a[b]]",
        "[a&&b]",
        "[]a]",
        "a]",
        "x{,3}",
        "a{",
        r"\A",
        r"\x41",
        "[abc",
        "(abc",
    ] {
        assert!(compile_pattern(bad).is_err(), "{bad} should be refused");
    }
    // `.` stops at any line break, as in a browser; \d is ASCII
    let re = compile_pattern("^.$").unwrap();
    assert!(re.is_match("a") && !re.is_match("\r") && !re.is_match("\u{2028}"));
    let re = compile_pattern(r"^\d$").unwrap();
    assert!(re.is_match("7") && !re.is_match("\u{0663}"));
    let c = codes_after(|v| v["messages"][0]["fields"][2]["pattern"] = json!(r"^\S+$"));
    assert!(
        has(
            &c,
            "E_SETTINGS_PATTERN",
            "messages[ChatSettings].fields[webhook_url].pattern"
        ),
        "{c:?}"
    );
}

#[test]
fn settings_options_belong_to_settings_fields() {
    let c = codes_after(|v| {
        v["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "name": "Other", "kind": "message",
            "fields": [field("n", "int32", json!({"max": 3}))] }))
    });
    assert!(
        has(&c, "E_FIELD_VALIDATOR_SCOPE", "messages[Other].fields[n]"),
        "{c:?}"
    );
}

#[test]
fn the_form_has_every_field_once_with_its_control() {
    let f = "settings.form.sections";
    let c =
        codes_after(|v| v["settings"]["form"]["sections"][1]["inputs"][0]["name"] = json!("nope"));
    assert!(
        has(&c, "E_SETTINGS_FORM_FIELD", &format!("{f}[1].inputs[0]")),
        "{c:?}"
    );
    assert!(has(&c, "E_SETTINGS_FORM_FIELD", "settings.form"), "{c:?}");
    let c = codes_after(|v| {
        v["settings"]["form"]["sections"][0]["inputs"]
            .as_array_mut()
            .unwrap()
            .push(input("tone", "radio_cards"))
    });
    assert!(
        has(&c, "E_SETTINGS_FORM_FIELD", &format!("{f}[1].inputs[1]")),
        "{c:?}"
    );
    let c = codes_after(|v| {
        v["settings"]["form"]["sections"][2]["inputs"][1]["control"] = json!("text")
    });
    assert!(
        has(
            &c,
            "E_SETTINGS_CONTROL",
            &format!("{f}[2].inputs[1].control")
        ),
        "{c:?}"
    );
    let c = codes_after(|v| {
        v["settings"]["form"]["sections"][2]["inputs"][0]["control"] = json!("secret")
    });
    assert!(
        has(
            &c,
            "E_SETTINGS_CONTROL",
            &format!("{f}[2].inputs[0].control")
        ),
        "{c:?}"
    );
    let c = codes_after(|v| {
        v["settings"]["form"]["sections"][1]["inputs"][1]["options"] =
            json!([{ "value": "shout", "label": "Shout" }])
    });
    assert!(
        has(
            &c,
            "E_SETTINGS_CONTROL",
            &format!("{f}[1].inputs[1].options")
        ),
        "{c:?}"
    );
}

#[test]
fn settings_have_limits() {
    let c = codes_after(|v| {
        let fields = v["messages"][0]["fields"].as_array_mut().unwrap();
        for i in 0..17 {
            fields.push(field(
                &format!("key_{i}"),
                "secret",
                json!({"optional": true}),
            ));
        }
        for i in 0..42 {
            fields.push(field(&format!("n_{i}"), "int32", json!({})));
        }
    });
    let limits: Vec<_> = c.iter().filter(|(c, _)| *c == "E_SETTINGS_LIMIT").collect();
    assert_eq!(limits.len(), 2, "{c:?}");
}

#[test]
fn a_required_field_without_a_default_is_warned_about() {
    let mut v = chat();
    v["messages"][0]["fields"][1]
        .as_object_mut()
        .unwrap()
        .remove("default");
    let w = warnings(&desc(&v));
    assert_eq!(w.len(), 1);
    assert_eq!(
        (w[0].code, w[0].path.as_str()),
        (
            "W_SETTINGS_REQUIRED_NO_DEFAULT",
            "messages[ChatSettings].fields[message_max_chars]"
        )
    );
    assert!(warnings(&desc(&chat())).is_empty());
}

#[test]
fn values_are_checked_type_then_required_then_validators() {
    let d = desc(&chat());
    let errs = |v: Value| validate_settings_value(&d, &map(v)).unwrap_err();
    let one = |v: Value| {
        let e = errs(v);
        assert_eq!(e.len(), 1, "{e:?}");
        (e[0].name.clone(), e[0].message.clone())
    };
    assert_eq!(
        one(json!({"thread_warm_hours": 49})),
        ("thread_warm_hours".into(), "From 1 to 48".into())
    );
    assert_eq!(
        one(json!({"message_max_chars": 20001})),
        ("message_max_chars".into(), "From 100 to 20,000".into())
    );
    assert_eq!(
        one(json!({"thread_warm_hours": "four"})),
        ("thread_warm_hours".into(), "Enter a whole number".into())
    );
    assert_eq!(
        one(json!({"webhook_url": "ftp://x.io"})),
        ("webhook_url".into(), "Not a URL".into())
    );
    assert_eq!(
        one(json!({"webhook_token": "x".repeat(201)})),
        ("webhook_token".into(), "At most 200 characters".into())
    );
    assert_eq!(
        one(json!({"tone": "shout"})),
        ("tone".into(), "Choose one of the options".into())
    );
    assert_eq!(
        one(json!({"watchers": ["u1", "u2", "u3", "u4"]})),
        ("watchers".into(), "At most 3 items".into())
    );
    assert_eq!(
        one(json!({"extra": 1})),
        ("extra".into(), "Not a setting".into())
    );
    // a field's own message replaces the platform's text
    let mut v = chat();
    v["messages"][0]["fields"][0]["message"] = json!("Between an hour and two days");
    let d2 = desc(&v);
    let e = validate_settings_value(&d2, &map(json!({"thread_warm_hours": 0}))).unwrap_err();
    assert_eq!(e[0].message, "Between an hour and two days");
    // required: a field with no value and no default
    let mut v = chat();
    v["messages"][0]["fields"][1]
        .as_object_mut()
        .unwrap()
        .remove("default");
    let d3 = desc(&v);
    let e = validate_settings_value(&d3, &map(json!({"message_max_chars": null}))).unwrap_err();
    assert_eq!(e, vec![FieldError::new("message_max_chars", "Required")]);

    // what passes comes back normalized; an unset value is not in it
    let ok = validate_settings_value(
        &d,
        &map(
            json!({"thread_warm_hours": "4", "webhook_url": "https://hooks.example.com/x",
                    "webhook_token": "sk-abc", "watchers": [], "tone": null}),
        ),
    )
    .unwrap();
    assert_eq!(
        Value::Object(ok),
        json!({"thread_warm_hours": 4, "webhook_url": "https://hooks.example.com/x",
               "webhook_token": "sk-abc", "watchers": []})
    );
}

#[test]
fn type_errors_have_the_screens_texts() {
    let v = json!({
      "schema": 1, "id": "t", "version": "1.0.0",
      "enums": [{ "name": "Tone", "values": ["calm"] }],
      "messages": [{ "name": "S", "kind": "settings", "description": "", "fields": [
          field("n", "int32", json!({"optional": true, "message": "Own text"})),
          field("big", "int64", json!({"optional": true})),
          field("x", "double", json!({"optional": true})),
          field("price", "decimal", json!({"optional": true})),
          field("day", "date", json!({"optional": true})),
          field("tone", "enum:Tone", json!({"optional": true})),
          field("name", "string", json!({"optional": true})),
          field("key", "secret", json!({"optional": true})),
          field("on", "bool", json!({"optional": true})),
          field("who", "ref:platform.user", json!({"optional": true})),
          field("people", "ref:platform.user", json!({"repeated": true})),
          field("tags", "string", json!({"repeated": true})),
          field("tones", "enum:Tone", json!({"repeated": true})),
      ] }],
      "settings": { "message": "S" }
    });
    let d = desc(&v);
    let message = |name: &str, x: Value| {
        let mut m = Map::new();
        m.insert(name.into(), x);
        validate_settings_value(&d, &m).unwrap_err()[0]
            .message
            .clone()
    };
    // a field's own message replaces validator texts, never the type's
    assert_eq!(message("n", json!("four")), "Enter a whole number");
    assert_eq!(message("big", json!(1.5)), "Enter a whole number");
    assert_eq!(message("x", json!("a")), "Enter a number");
    assert_eq!(message("price", json!("1,5")), "Enter a number");
    assert_eq!(message("day", json!("2026-13-01")), "Enter a date");
    assert_eq!(message("tone", json!("loud")), "Choose one of the options");
    assert_eq!(message("name", json!(5)), "Enter text");
    assert_eq!(message("key", json!(true)), "Enter text");
    assert_eq!(message("on", json!("yes")), "Choose on or off");
    assert_eq!(message("who", json!(7)), "Choose people");
    assert_eq!(message("people", json!("u1")), "Choose people");
    assert_eq!(message("tags", json!("a")), "Enter text");
    assert_eq!(
        message("tones", json!(["calm", "x"])),
        "Choose one of the options"
    );
    assert_eq!(message("nope", json!(1)), "Not a setting");
    assert_eq!(
        message("name", json!("x".repeat(MAX_VALUE_BYTES))),
        "Too large (at most 64 KiB)"
    );
}

#[test]
fn every_validator_has_its_message() {
    let mut v = chat();
    let fields = v["messages"][0]["fields"].as_array_mut().unwrap();
    fields.push(field(
        "code",
        "string",
        json!({"optional": true, "pattern": "^[A-Z]{3}$", "min_len": 3}),
    ));
    fields.push(field(
        "mail",
        "string",
        json!({"optional": true, "format": "email"}),
    ));
    fields.push(field(
        "price",
        "decimal",
        json!({"optional": true, "min": "0.5"}),
    ));
    fields.push(field(
        "cap",
        "int64",
        json!({"optional": true, "max": 1500000}),
    ));
    fields.push(field(
        "from",
        "date",
        json!({"optional": true, "min": "2026-01-01", "max": "2026-12-31"}),
    ));
    fields.push(field(
        "tags",
        "string",
        json!({"repeated": true, "min_items": 1, "max_len": 3}),
    ));
    fields.push(field(
        "ratio",
        "double",
        json!({"optional": true, "max": 0.75}),
    ));
    let sections = v["settings"]["form"]["sections"][1]["inputs"]
        .as_array_mut()
        .unwrap();
    for (n, c) in [
        ("code", "text"),
        ("mail", "text"),
        ("price", "number"),
        ("cap", "number"),
        ("from", "date"),
        ("tags", "textarea"),
        ("ratio", "number"),
    ] {
        sections.push(input(n, c));
    }
    assert_eq!(codes(&v), vec![]);
    let d = desc(&v);
    let msg = |v: Value| {
        validate_settings_value(&d, &map(v))
            .unwrap_err()
            .into_iter()
            .map(|e| e.message)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        msg(json!({"code": "ab", "tags": ["x"]})),
        ["At least 3 characters"]
    );
    assert_eq!(
        msg(json!({"code": "abc", "tags": ["x"]})),
        ["Doesn't match the expected format"]
    );
    assert_eq!(
        msg(json!({"mail": "a@b", "tags": ["x"]})),
        ["Not an email address"]
    );
    assert_eq!(
        msg(json!({"price": "0.49", "tags": ["x"]})),
        ["At least 0.5"]
    );
    assert_eq!(
        msg(json!({"price": "abc", "tags": ["x"]})),
        ["Enter a number"]
    );
    assert_eq!(
        msg(json!({"cap": "1500001", "tags": ["x"]})),
        ["At most 1,500,000"]
    );
    assert_eq!(
        msg(json!({"from": "2027-01-01", "tags": ["x"]})),
        ["From 2026-01-01 to 2026-12-31"]
    );
    assert_eq!(
        msg(json!({"from": "2026-02-30", "tags": ["x"]})),
        ["Enter a date"]
    );
    assert_eq!(msg(json!({"tags": []})), ["At least 1 items"]);
    assert_eq!(msg(json!({"tags": ["long"]})), ["At most 3 characters"]);
    assert_eq!(msg(json!({"tags": ["x"], "ratio": 0.8})), ["At most 0.75"]);
    assert!(
        validate_settings_value(
            &d,
            &map(json!({"code": "ABC", "mail": "sarah@example.com", "price": 2, "cap": 7, "tags": ["x"],
                        "from": "2026-06-01", "ratio": 0.5}))
        )
        .is_ok()
    );
    let ok = validate_settings_value(&d, &map(json!({"price": 2, "tags": ["x"]}))).unwrap();
    assert_eq!(ok["price"], json!("2"));
}

#[test]
fn urls_and_emails() {
    for ok in [
        "https://a.io",
        "http://localhost:8080/x?y#z",
        "HTTPS://user@a.b/c",
        "http://[::1]/",
    ] {
        assert!(url_ok(ok), "{ok}");
    }
    for bad in [
        "a.io",
        "https://",
        "https:// a.io",
        "mailto:x@y.z",
        "https://a.io:x/",
        "https:///x",
    ] {
        assert!(!url_ok(bad), "{bad}");
    }
    for ok in ["a@b.co", "first.last+tag@sub.example.com"] {
        assert!(email_ok(ok), "{ok}");
    }
    for bad in ["a@b", "@b.co", "a@@b.co", "a b@c.de", "a@.co", "a@b.co."] {
        assert!(!email_ok(bad), "{bad}");
    }
}

#[test]
fn bounds_are_written_with_thousands_separators() {
    assert_eq!(display_bound(&json!(20000)), "20,000");
    assert_eq!(display_bound(&json!(-1500)), "-1,500");
    assert_eq!(display_bound(&json!("1234567.25")), "1,234,567.25");
    assert_eq!(display_bound(&json!(100)), "100");
    assert_eq!(display_bound(&json!("2026-01-01")), "2026-01-01");
}

#[test]
fn what_the_plugin_reads() {
    let d = desc(&chat());
    let saved = map(json!({"thread_warm_hours": 4, "webhook_token": "sk-1"}));
    let eff = effective(&d, &saved);
    assert_eq!(
        Value::Object(eff.clone()),
        json!({"thread_warm_hours": 4, "message_max_chars": 20000, "webhook_token": "sk-1", "tone": "calm"})
    );
    assert_eq!(
        Value::Object(to_plugin_json(&d, &eff)),
        json!({"threadWarmHours": 4, "messageMaxChars": 20000, "webhookToken": "sk-1", "tone": "calm"})
    );
    assert!(missing(&d, &saved).is_empty());
    let mut v = chat();
    v["messages"][0]["fields"][0]
        .as_object_mut()
        .unwrap()
        .remove("default");
    v["messages"][0]["fields"][1]["type"] = json!("int64");
    let d = desc(&v);
    assert_eq!(
        missing(&d, &Map::new()),
        vec!["thread_warm_hours".to_string()]
    );
    assert_eq!(
        to_plugin_json(&d, &effective(&d, &Map::new()))["messageMaxChars"],
        json!("20000")
    );
    assert_eq!(
        super::field(&d, "webhookUrl").map(|f| f.name.as_str()),
        Some("webhook_url")
    );
    assert_eq!(
        super::field(&d, "webhook_url").map(|f| f.name.as_str()),
        Some("webhook_url")
    );
}

#[test]
fn saved_values_are_brought_to_a_new_version() {
    let mut v = chat();
    let fields = v["messages"][0]["fields"].as_array_mut().unwrap();
    fields.retain(|f| f["name"] != "webhook_url");
    fields[0]["max"] = json!(24);
    fields[0]["default"] = json!(2);
    v["settings"]["form"]["sections"][2]["inputs"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    let new = desc(&v);
    let saved = map(json!({"thread_warm_hours": 30, "message_max_chars": 500,
                           "webhook_url": "https://a.io", "tone": "calm"}));
    let c = convert(&new, &saved);
    assert_eq!(c.dropped, vec!["webhook_url".to_string()]);
    assert_eq!(c.reset, vec!["thread_warm_hours".to_string()]);
    assert_eq!(
        Value::Object(c.values),
        json!({"message_max_chars": 500, "tone": "calm"})
    );
}

#[test]
fn settings_changes_are_classified() {
    let old = desc(&chat());
    let changed = |f: &dyn Fn(&mut Value)| {
        let mut v = chat();
        f(&mut v);
        v["version"] = json!("1.1.0");
        let new = desc(&v);
        changes(&old, &new)
            .into_iter()
            .map(|c| (c.code, c.breaking))
            .collect::<Vec<_>>()
    };
    // nothing
    assert_eq!(changed(&|_| {}), vec![]);
    // labels, sections, the form, the hook: nothing
    assert_eq!(
        changed(&|v| {
            v["messages"][0]["fields"][0]["label"] = json!("Hours");
            v["settings"]["on_change"] = json!(false);
        }),
        vec![]
    );
    // a new optional field: minor; a new required one without a default: major
    assert_eq!(
        changed(&|v| v["messages"][0]["fields"]
            .as_array_mut()
            .unwrap()
            .push(field("note", "string", json!({"optional": true})))),
        vec![("W_SETTINGS_FIELD_ADDED", false)]
    );
    assert_eq!(
        changed(&|v| v["messages"][0]["fields"]
            .as_array_mut()
            .unwrap()
            .push(field("note", "string", json!({})))),
        vec![("E_SETTINGS_REQUIRED_ADDED", true)]
    );
    // removed, retyped
    assert_eq!(
        changed(&|v| {
            v["messages"][0]["fields"].as_array_mut().unwrap().remove(4);
        }),
        vec![("E_SETTINGS_FIELD_REMOVED", true)]
    );
    assert_eq!(
        changed(&|v| v["messages"][0]["fields"][2]["type"] = json!("int32")),
        vec![("E_SETTINGS_FIELD_TYPE_CHANGED", true)]
    );
    // validators: tightened is major, loosened minor
    assert_eq!(
        changed(&|v| v["messages"][0]["fields"][0]["max"] = json!(24)),
        vec![("E_SETTINGS_VALIDATOR_TIGHTENED", true)]
    );
    assert_eq!(
        changed(&|v| {
            v["messages"][0]["fields"][0]["max"] = json!(72);
            v["messages"][0]["fields"][3]
                .as_object_mut()
                .unwrap()
                .remove("max_len");
        }),
        vec![
            ("W_SETTINGS_VALIDATOR_LOOSENED", false),
            ("W_SETTINGS_VALIDATOR_LOOSENED", false)
        ]
    );
    assert_eq!(
        changed(&|v| v["messages"][0]["fields"][2]["pattern"] = json!("^https://")),
        vec![("E_SETTINGS_VALIDATOR_TIGHTENED", true)]
    );
    // a default dropped from a required field: it needs setup now
    assert_eq!(
        changed(&|v| {
            v["messages"][0]["fields"][1]
                .as_object_mut()
                .unwrap()
                .remove("default");
        }),
        vec![("E_SETTINGS_REQUIRED_ADDED", true)]
    );
    assert_eq!(
        changed(&|v| v["messages"][0]["fields"][1]["default"] = json!(10000)),
        vec![("W_SETTINGS_DEFAULT_CHANGED", false)]
    );
    // settings gone: major
    assert_eq!(
        changed(&|v| {
            v.as_object_mut().unwrap().remove("settings");
            v["messages"][0]["kind"] = json!("message");
            for f in v["messages"][0]["fields"].as_array_mut().unwrap() {
                let o = f.as_object_mut().unwrap();
                for k in [
                    "label",
                    "default",
                    "min",
                    "max",
                    "section",
                    "format",
                    "max_len",
                    "max_items",
                ] {
                    o.remove(k);
                }
            }
        }),
        vec![("E_SETTINGS_REMOVED", true)]
    );
}

#[test]
fn the_publish_check_and_the_version_bump_see_settings() {
    let old = desc(&chat());
    let mut v = chat();
    v["version"] = json!("1.1.0");
    v["messages"][0]["fields"][0]["max"] = json!(72);
    let loosened = desc(&v);
    let (bump, why) = lock::bump_needed(&old, &loosened);
    assert_eq!(bump, lock::Bump::Minor, "{why:?}");
    v["messages"][0]["fields"][0]["max"] = json!(12);
    v["messages"][0]["fields"][0]["default"] = json!(2);
    let tightened = desc(&v);
    let (bump, why) = lock::bump_needed(&old, &tightened);
    assert_eq!(bump, lock::Bump::Major);
    assert!(
        why.iter()
            .any(|w| w.starts_with("Resets thread_warm_hours")),
        "{why:?}"
    );
    assert!(
        lock::compare(&old, &tightened)
            .iter()
            .any(|c| c.code == "E_SETTINGS_VALIDATOR_TIGHTENED")
    );
}

#[test]
fn a_derived_form_follows_the_fields() {
    let d = desc(&chat());
    let m = settings_message(&d).unwrap();
    let mut fields = m.fields.clone();
    fields[1].description = "The longest message a member may send.".into();
    fields[2].placeholder = Some("https://".into());
    let form = derive_form(&fields, &d.enums);
    assert_eq!(
        serde_json::to_value(&form).unwrap(),
        json!({ "sections": [
          { "title": "Agents", "inputs": [
            { "name": "thread_warm_hours", "control": "number", "label": "Thread stays with its agent, hours" } ] },
          { "inputs": [
            { "name": "message_max_chars", "control": "number", "label": "Message max chars",
              "help": "The longest message a member may send." },
            { "name": "tone", "control": "select", "label": "Tone",
              "options": [{ "value": "calm", "label": "Calm" },
                          { "value": "loud_and_clear", "label": "Loud and clear" }] },
            { "name": "watchers", "control": "member", "label": "Watchers" } ] },
          { "title": "Integrations", "inputs": [
            { "name": "webhook_url", "control": "text", "label": "Webhook url", "placeholder": "https://" },
            { "name": "webhook_token", "control": "secret", "label": "Webhook token" } ] }
        ] })
    );
    // a required field is marked; a multiline string is a text area
    let mut f = fields[2].clone();
    f.optional = false;
    f.multiline = true;
    f.format = None;
    let form = derive_form(&[f], &[]);
    let i = &form.sections[0].inputs[0];
    assert_eq!((i.control, i.required), (Control::Textarea, true));
    // a laid-out form gets what its fields carry
    let laid: SettingsForm = serde_json::from_value(json!({ "sections": [{ "inputs": [
        { "name": "tone", "control": "radio_cards", "label": "How loud" },
        { "name": "message_max_chars", "control": "number", "label": "Longest", "help": "Own help" } ] }] }))
    .unwrap();
    let done = complete_form(laid, &fields, &d.enums);
    let v = serde_json::to_value(&done).unwrap();
    assert_eq!(
        v["sections"][0]["inputs"][0]["options"][1]["label"],
        json!("Loud and clear")
    );
    assert_eq!(v["sections"][0]["inputs"][1]["help"], json!("Own help"));
}

#[test]
fn the_hook_has_a_reserved_method() {
    assert_eq!(
        crate::settings_check_path("chat"),
        "/chat.v1.Chat/_SettingsCheck"
    );
    assert!(crate::is_reserved_method(&crate::settings_check_path(
        "chat"
    )));
}
