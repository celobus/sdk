//! Settings: what `--describe` writes, the cache `ctx.settings()` reads, and the check hook.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::prelude::*;

#[derive(Message, Clone, Debug, PartialEq)]
pub enum Tone {
    Calm,
    LoudAndClear,
}

/// How the chat treats agents in threads.
#[derive(Message, Clone, Debug, PartialEq)]
#[message(settings)]
pub struct ChatSettings {
    /// After this many hours without messages a thread no longer sticks to the agent that answered in it.
    #[field(
        label = "Thread stays with its agent, hours",
        default = 2,
        min = 1,
        max = 48,
        section = "Agents"
    )]
    pub thread_warm_hours: i32,
    /// The longest message a member may send.
    #[field(
        default = 20000,
        min = 100,
        max = 20000,
        message = "From 100 to 20,000 characters"
    )]
    pub message_max_chars: i64,
    #[field(
        label = "Webhook URL",
        url,
        section = "Integrations",
        placeholder = "https://"
    )]
    pub webhook_url: Option<String>,
    #[field(section = "Integrations", max_len = 200)]
    pub webhook_token: Option<Secret>,
    #[field(default = "calm")]
    pub tone: Tone,
    #[field(max_items = 3)]
    pub watchers: Vec<Id<platform::User>>,
    #[field(multiline, pattern = "^[A-Z]", min_len = 1)]
    pub greeting: String,
    pub price: Option<Decimal>,
}

#[derive(Message, Clone, Debug)]
pub struct PingRequest {}

#[derive(Message, Clone, Debug)]
pub struct PingReply {
    pub hours: i32,
}

async fn ping(ctx: Ctx, _: PingRequest) -> Result<PingReply> {
    Ok(PingReply {
        hours: ctx.settings::<ChatSettings>()?.thread_warm_hours,
    })
}

fn plugin() -> Plugin {
    Plugin::new("chat", "1.0.0")
        .endpoint(Endpoint::new(ping))
        .settings(Settings::<ChatSettings>::new())
}

fn describe(p: &Plugin) -> Value {
    serde_json::from_str(&p.describe_json().unwrap()).unwrap()
}

#[test]
fn describe_writes_the_settings_and_their_fields_options() {
    let d = describe(&plugin());
    let m = d["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "ChatSettings")
        .unwrap();
    assert_eq!(m["kind"], json!("settings"));
    assert_eq!(
        m["fields"][0],
        json!({ "name": "thread_warm_hours", "number": 1, "type": "int32", "repeated": false, "optional": false,
                "description": "After this many hours without messages a thread no longer sticks to the agent that answered in it.",
                "label": "Thread stays with its agent, hours", "default": 2, "min": 1, "max": 48,
                "section": "Agents" })
    );
    assert_eq!(
        m["fields"][1]["message"],
        json!("From 100 to 20,000 characters")
    );
    assert_eq!(m["fields"][2]["format"], json!("url"));
    assert_eq!(m["fields"][3]["type"], json!("secret"));
    assert_eq!(m["fields"][4]["default"], json!("calm"));
    assert_eq!(m["fields"][6]["multiline"], json!(true));
    assert_eq!(m["fields"][6]["pattern"], json!("^[A-Z]"));
    assert_eq!(
        d["settings"],
        json!({ "message": "ChatSettings", "on_change": false, "form": { "sections": [
          { "title": "Agents", "inputs": [
            { "name": "thread_warm_hours", "control": "number", "label": "Thread stays with its agent, hours",
              "help": "After this many hours without messages a thread no longer sticks to the agent that answered in it." } ] },
          { "inputs": [
            { "name": "message_max_chars", "control": "number", "label": "Message max chars",
              "help": "The longest message a member may send." },
            { "name": "tone", "control": "select", "label": "Tone",
              "options": [{ "value": "calm", "label": "Calm" }, { "value": "loud_and_clear", "label": "Loud and clear" }] },
            { "name": "watchers", "control": "member", "label": "Watchers" },
            { "name": "greeting", "control": "textarea", "label": "Greeting", "required": true },
            { "name": "price", "control": "number", "label": "Price" } ] },
          { "title": "Integrations", "inputs": [
            { "name": "webhook_url", "control": "text", "label": "Webhook URL", "placeholder": "https://" },
            { "name": "webhook_token", "control": "secret", "label": "Webhook token" } ] }
        ] } })
    );
    // what the platform reads back checks
    let checked = celobus_descriptor::check(d.to_string().as_bytes()).unwrap();
    assert!(checked.descriptor.settings.is_some());
    assert_eq!(
        celobus_descriptor::settings::warnings(&checked.descriptor)
            .iter()
            .map(|w| w.path.as_str())
            .collect::<Vec<_>>(),
        ["messages[ChatSettings].fields[greeting]"]
    );
}

#[test]
fn a_form_laid_out_by_hand_gets_what_its_fields_carry() {
    let p = Plugin::new("chat", "1.0.0").settings(
        Settings::<ChatSettings>::new()
            .on_change(|_: Ctx, _: SettingsChange<ChatSettings>| async { Ok(()) })
            .form(|f| {
                f.section("Messages");
                f.number(ChatSettings::message_max_chars, "Longest message");
                f.textarea(ChatSettings::greeting, "Greeting")
                    .help("Shown first");
                f.section("Other");
                f.number(ChatSettings::thread_warm_hours, "Hours");
                f.radio(ChatSettings::tone, "Tone");
                f.member(ChatSettings::watchers, "Watchers");
                f.text(ChatSettings::webhook_url, "URL");
                f.secret(ChatSettings::webhook_token, "Token");
                f.number(ChatSettings::price, "Price");
            }),
    );
    let d = describe(&p);
    let s = &d["settings"];
    assert_eq!(s["on_change"], json!(true));
    assert_eq!(s["form"]["sections"].as_array().unwrap().len(), 2);
    let messages = &s["form"]["sections"][0];
    assert_eq!(messages["title"], json!("Messages"));
    assert_eq!(
        messages["inputs"][0],
        json!({ "name": "message_max_chars", "control": "number", "label": "Longest message",
                "help": "The longest message a member may send." })
    );
    assert_eq!(
        messages["inputs"][1],
        json!({ "name": "greeting", "control": "textarea", "label": "Greeting", "help": "Shown first",
                "required": true })
    );
    assert_eq!(
        s["form"]["sections"][1]["inputs"][1]["options"][1],
        json!({ "value": "loud_and_clear", "label": "Loud and clear" })
    );
    assert!(celobus_descriptor::check(d.to_string().as_bytes()).is_ok());
}

#[test]
fn describe_refuses_a_form_without_every_field() {
    let p = Plugin::new("chat", "1.0.0").settings(Settings::<ChatSettings>::new().form(|f| {
        f.number(ChatSettings::thread_warm_hours, "Hours");
        f.text(ChatSettings::thread_warm_hours, "Twice");
    }));
    let errors = p.check();
    assert!(
        errors.iter().any(|e| e.code == "E_SETTINGS_FORM_FIELD"
            && e.message.contains("greeting is not in the form")),
        "{errors:?}"
    );
    assert!(
        errors.iter().any(|e| e.code == "E_SETTINGS_CONTROL"),
        "{errors:?}"
    );
}

#[test]
fn a_secret_stays_out_of_logs() {
    let s = Secret::new("sk-live-123");
    assert_eq!(format!("{s:?}"), "Secret(••••)");
    assert_eq!(s.to_string(), "Secret(••••)");
    assert_eq!(s.expose(), "sk-live-123");
    let settings = ChatSettings {
        webhook_token: Some(s),
        ..settings()
    };
    assert!(!format!("{settings:?}").contains("sk-live"));
    assert_eq!(settings.to_json()["webhookToken"], json!("sk-live-123"));
}

fn settings() -> ChatSettings {
    ChatSettings {
        thread_warm_hours: 4,
        message_max_chars: 500,
        webhook_url: None,
        webhook_token: None,
        tone: Tone::LoudAndClear,
        watchers: vec![],
        greeting: "Hi".into(),
        price: None,
    }
}

#[test]
fn a_test_context_has_the_settings_it_is_given() {
    let ctx = Ctx::test("chat", "1.0.0");
    let err = ctx.settings::<ChatSettings>().unwrap_err();
    assert_eq!(err.status(), 500);
    let ctx = ctx.with_settings(settings());
    let s = ctx.settings::<ChatSettings>().unwrap();
    assert_eq!(*s, settings());
    // read once, then shared
    assert!(Arc::ptr_eq(&s, &ctx.settings::<ChatSettings>().unwrap()));
    // the contexts made from it see them too
    let other = ctx.with_caller(Caller::test_user("u-1"));
    assert_eq!(
        other.settings::<ChatSettings>().unwrap().thread_warm_hours,
        4
    );
}

#[test]
fn the_cache_fills_defaults_and_names_what_is_missing() {
    let cache = crate::settings::SettingsCache::declared(Some("ChatSettings"));
    // nothing sent yet: the defaults, and the required greeting is missing
    let err = cache.get::<ChatSettings>().unwrap_err();
    assert_eq!((err.status(), err.code()), (409, "settings_required"));
    assert_eq!(err.details()["missing"], json!(["greeting"]));
    assert_eq!(err.fields().collect::<Vec<_>>(), [("greeting", "Required")]);

    assert!(cache.update(3, br#"{"greeting": "Hi", "threadWarmHours": 6}"#, true));
    let s = cache.get::<ChatSettings>().unwrap();
    assert_eq!(
        (s.thread_warm_hours, s.message_max_chars, s.tone.clone()),
        (6, 20000, Tone::Calm)
    );
    // an older change is stale; a newer one replaces them
    assert!(!cache.update(2, br#"{"greeting": "Old"}"#, false));
    assert_eq!(cache.get::<ChatSettings>().unwrap().greeting, "Hi");
    assert!(cache.update(
        4,
        br#"{"greeting": "New", "messageMaxChars": "300"}"#,
        false
    ));
    let s = cache.get::<ChatSettings>().unwrap();
    assert_eq!((s.greeting.as_str(), s.message_max_chars), ("New", 300));
    // a value of the wrong type is an error, never a guess
    assert!(cache.update(5, br#"{"greeting": "X", "threadWarmHours": "many"}"#, false));
    assert_eq!(cache.get::<ChatSettings>().unwrap_err().status(), 500);
    // another message than the declared one
    #[derive(Message, Clone, Debug)]
    #[message(settings)]
    pub struct Other {
        pub n: Option<i32>,
    }
    assert_eq!(cache.get::<Other>().unwrap_err().status(), 500);
    // a registration replaces them whatever its revision
    assert!(cache.update(0, br#"{"greeting": "Again"}"#, true));
    assert_eq!(cache.get::<ChatSettings>().unwrap().greeting, "Again");
}

#[tokio::test]
async fn the_check_hook_is_a_system_request() {
    use crate::session::tests_support::{dispatch_member, dispatch_system};
    type Seen = std::sync::Mutex<Vec<(Option<i32>, i32)>>;
    let seen: Arc<Seen> = Arc::default();
    let s = seen.clone();
    let p = Plugin::new("chat", "1.0.0").settings(Settings::<ChatSettings>::new().on_change(
        move |ctx: Ctx, change: SettingsChange<ChatSettings>| {
            let s = s.clone();
            async move {
                assert_eq!(ctx.caller().kind(), CallerKind::System);
                if change.current.webhook_token.is_some() && change.current.webhook_url.is_none() {
                    return Err(Error::invalid("a token needs a URL")
                        .field("webhook_url", "Set the URL the token is for"));
                }
                if change.current.greeting == "Boom" {
                    return Err(Error::new(422, "unreachable", "The service did not answer"));
                }
                s.lock().unwrap().push((
                    change.previous.map(|p| p.thread_warm_hours),
                    change.current.thread_warm_hours,
                ));
                Ok(())
            }
        },
    ));
    let path = celobus_descriptor::settings_check_path("chat");
    assert_eq!(path, "/chat.v1.Chat/_SettingsCheck");
    let service = crate::session::Service::new(p).unwrap();

    let body = json!({ "previous": {"greeting": "Hi"}, "current": {"greeting": "Hi", "threadWarmHours": 8} });
    let (status, b) = dispatch_member(&service, &path, &body).await;
    assert_eq!(
        (status, b["code"].as_str()),
        (404, Some("unknown_endpoint"))
    );

    let (status, b) = dispatch_system(&service, &path, &body).await;
    assert_eq!((status, b), (200, json!({})));
    let body = json!({ "previous": {}, "current": {"greeting": "Hi", "threadWarmHours": 9} });
    assert_eq!(dispatch_system(&service, &path, &body).await.0, 200);
    assert_eq!(seen.lock().unwrap().as_slice(), &[(Some(2), 8), (None, 9)]);

    let body = json!({ "previous": null, "current": {"greeting": "Hi", "webhookToken": "sk-1"} });
    let (status, b) = dispatch_system(&service, &path, &body).await;
    assert_eq!(status, 400);
    assert_eq!(
        b["details"]["fields"],
        json!({"webhook_url": "Set the URL the token is for"})
    );
    let body = json!({ "current": {"greeting": "Boom"} });
    let (status, b) = dispatch_system(&service, &path, &body).await;
    assert_eq!(
        (status, b["message"].as_str()),
        (422, Some("The service did not answer"))
    );
}
