//! Dispatch and error mapping without the platform, and a whole session against a fake platform.

use std::sync::Arc;
use std::time::Duration;

use celobus_proto as pb;
use pb::{celld_message, plugin_message, syscall, syscall_result};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_stream::wrappers::{ReceiverStream, UnixListenerStream};

use super::*;
use crate::prelude::*;

#[derive(Message, Clone, Debug)]
pub struct EchoRequest {
    pub text: String,
    pub times: i64,
}

#[derive(Message, Clone, Debug)]
pub struct EchoReply {
    pub text: String,
    pub caller: String,
}

#[derive(Message, Clone, Debug)]
pub struct FailRequest {
    pub how: String,
}

#[derive(Message, Clone, Debug)]
pub struct CountRequest {}

/// A greeting counted: a row each.
#[derive(Entity, Clone, Debug)]
#[entity(key = i64)]
pub struct Tally {
    pub id: Id<Tally>,
    pub n: i64,
}

#[derive(Message, Clone, Debug)]
pub struct CountReply {
    pub count: i64,
    pub greeting: String,
}

#[derive(Message, Clone, Debug)]
#[message(response = GreetReply)]
pub struct GreetRequest {
    pub name: String,
}

#[derive(Message, Clone, Debug)]
pub struct GreetReply {
    pub text: String,
}

pub const NOT_CANCELLABLE: ErrorCode = ErrorCode::new("not_cancellable", 409);

async fn echo(ctx: Ctx, req: EchoRequest) -> Result<EchoReply> {
    Ok(EchoReply {
        text: req.text.repeat(req.times as usize),
        caller: ctx.caller().user()?.to_string(),
    })
}

async fn fail(_: Ctx, req: FailRequest) -> Result<EchoReply> {
    match req.how.as_str() {
        "invalid" => Err(Error::invalid("bad input").field("how", "not this")),
        "domain" => Err(NOT_CANCELLABLE.error("order SO-1 is shipped")),
        "internal" => Err(anyhow::anyhow!("secret database detail").into()),
        "panic" => panic!("boom"),
        "slow" => {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Err(Error::internal("unreachable"))
        }
        _ => Err(Error::not_found("no such thing")),
    }
}

/// A counter of rows and a dependency call: the syscalls of a session.
async fn count(ctx: Ctx, _: CountRequest) -> Result<CountReply> {
    let count = ctx.db::<Tally>().count().await? as i64 + 1;
    ctx.db::<Tally>()
        .create(Tally {
            id: Id::none(),
            n: count,
        })
        .await?;
    let greeting = ctx
        .call(
            "greeter",
            &GreetRequest {
                name: "Sarah".into(),
            },
        )
        .await?
        .text;
    Ok(CountReply { count, greeting })
}

fn plugin() -> Plugin {
    Plugin::new("echo", "1.0.0")
        .depends_on("greeter", ["Greet"])
        .endpoint(Endpoint::new(echo))
        .endpoint(Endpoint::new(fail).timeout(Duration::from_millis(200)))
        .endpoint(Endpoint::new(count))
}

fn request(
    id: u64,
    method: &str,
    path: &str,
    content_type: Option<&str>,
    body: &str,
) -> pb::Request {
    pb::Request {
        id,
        method: method.into(),
        path: path.into(),
        headers: content_type
            .map(|ct| {
                vec![pb::Header {
                    name: "Content-Type".into(),
                    value: ct.into(),
                }]
            })
            .unwrap_or_default(),
        body: body.as_bytes().to_vec().into(),
        principal: Some(pb::Principal {
            user_id: "u-42".into(),
            kind: pb::principal::Kind::Person as i32,
            display_name: "Sarah Bennett".into(),
            roles: vec!["workspace.member".into(), "echo.clerk".into()],
            groups: vec![],
            scopes: vec!["echo:write".into()],
        }),
        ..Default::default()
    }
}

fn body(resp: &pb::Response) -> Value {
    serde_json::from_slice(&resp.body).unwrap()
}

async fn call(
    service: &Service,
    method: &str,
    path: &str,
    content_type: Option<&str>,
    body_text: &str,
) -> (u32, Value) {
    let resp = service
        .dispatch(
            Arc::new(Link::detached()),
            request(9, method, path, content_type, body_text),
        )
        .await;
    assert_eq!(resp.id, 9);
    assert!(
        resp.headers
            .iter()
            .any(|h| h.name == "content-type" && h.value == "application/json")
    );
    (resp.status, body(&resp))
}

#[tokio::test]
async fn dispatches_and_maps_errors() {
    let service = Service::new(plugin()).unwrap();
    let echo = "/echo.v1.Echo/Echo";
    let fail = "/echo.v1.Echo/Fail";

    assert_eq!(
        call(
            &service,
            "POST",
            echo,
            Some("application/json; charset=utf-8"),
            r#"{"text":"ab","times":"2"}"#
        )
        .await,
        (200, json!({"text": "abab", "caller": "u-42"}))
    );
    // no content type and an empty body: an empty message
    assert_eq!(
        call(&service, "POST", echo, None, "").await,
        (200, json!({"text": "", "caller": "u-42"}))
    );

    let (status, b) = call(&service, "POST", "/echo.v1.Echo/Nope", None, "{}").await;
    assert_eq!(
        (status, b["code"].as_str()),
        (404, Some("unknown_endpoint"))
    );
    let (status, b) = call(&service, "GET", echo, None, "").await;
    assert_eq!(
        (status, b["code"].as_str()),
        (405, Some("method_not_allowed"))
    );
    let (status, b) = call(&service, "POST", echo, Some("application/x-protobuf"), "").await;
    assert_eq!(
        (status, b["code"].as_str()),
        (415, Some("unsupported_media_type"))
    );
    let (status, b) = call(&service, "POST", echo, None, "{not json").await;
    assert_eq!((status, b["code"].as_str()), (400, Some("invalid")));
    let (status, b) = call(&service, "POST", echo, None, r#"{"times":"x"}"#).await;
    assert_eq!(status, 400);
    assert_eq!(
        b["details"],
        json!({"fields": {"times": "must be a 64-bit integer"}})
    );

    let cases = [
        (
            "invalid",
            400,
            json!({"code": "invalid", "message": "bad input", "details": {"fields": {"how": "not this"}}}),
        ),
        (
            "domain",
            409,
            json!({"code": "not_cancellable", "message": "order SO-1 is shipped", "details": {}}),
        ),
        (
            "internal",
            500,
            json!({"code": "internal", "message": "internal error", "details": {}}),
        ),
        (
            "panic",
            500,
            json!({"code": "internal", "message": "internal error", "details": {}}),
        ),
        (
            "missing",
            404,
            json!({"code": "not_found", "message": "no such thing", "details": {}}),
        ),
    ];
    for (how, status, want) in cases {
        let got = call(
            &service,
            "POST",
            fail,
            None,
            &json!({ "how": how }).to_string(),
        )
        .await;
        assert_eq!(got, (status, want), "{how}");
    }
    let (status, b) = call(&service, "POST", fail, None, r#"{"how":"slow"}"#).await;
    assert_eq!(
        (status, b["code"].as_str()),
        (504, Some("deadline_exceeded"))
    );

    // no caller: `user()?` is 401
    let mut req = request(3, "POST", echo, None, "{}");
    req.principal = None;
    let resp = service.dispatch(Arc::new(Link::detached()), req).await;
    assert_eq!(
        (resp.status, body(&resp)["code"].clone()),
        (401, json!("unauthenticated"))
    );

    // syscalls without a session: 503
    let (status, b) = call(&service, "POST", "/echo.v1.Echo/Count", None, "{}").await;
    assert_eq!((status, b["code"].as_str()), (503, Some("unavailable")));
}

/// The run-change hook: a request of the platform, with no caller; one from a caller is refused as unknown.
#[tokio::test]
async fn the_run_change_hook_is_a_system_request() {
    let seen: Arc<std::sync::Mutex<Vec<RunChange>>> = Arc::default();
    let s = seen.clone();
    let plugin = plugin().on_run_change(move |ctx: Ctx, change: RunChange| {
        let s = s.clone();
        async move {
            assert_eq!(ctx.caller().kind(), CallerKind::System);
            if change.turn == 9 {
                return Err(Error::internal("try again"));
            }
            s.lock().unwrap().push(change);
            Ok(())
        }
    });
    let d = plugin.describe_json().unwrap();
    let d: Value = serde_json::from_str(&d).unwrap();
    assert_eq!(d["runs"], json!({"on_change": true}));
    assert!(serde_json::from_value::<celobus_descriptor::Descriptor>(d).is_ok());
    let service = Service::new(plugin).unwrap();
    let path = "/echo.v1.Echo/_RunChange";
    assert_eq!(path, celobus_descriptor::run_change_path("echo"));
    let change = json!({"run_id": "run-1", "agent": "assistant", "plugin": "crm", "status": "idle",
        "turn": 2, "last_seq": 14, "topic": "conversation", "topic_key": "{\"channel\":\"c-1\"}", "ended": false});

    // from a member (it came from outside): no such endpoint
    let (status, b) = call(&service, "POST", path, None, &change.to_string()).await;
    assert_eq!(
        (status, b["code"].as_str()),
        (404, Some("unknown_endpoint"))
    );
    assert!(seen.lock().unwrap().is_empty());

    let system = |body: String| {
        let mut req = request(5, "POST", path, Some("application/json"), &body);
        req.principal = None;
        req
    };
    let resp = service
        .dispatch(Arc::new(Link::detached()), system(change.to_string()))
        .await;
    assert_eq!((resp.status, body(&resp)), (200, json!({})));
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        &[RunChange {
            run_id: "run-1".into(),
            agent: "assistant".into(),
            plugin: "crm".into(),
            status: AgentRunStatus::Idle,
            turn: 2,
            last_seq: 14,
            topic: "conversation".into(),
            topic_key: r#"{"channel":"c-1"}"#.into(),
            ended: false,
        }]
    );
    // an error answers non-2xx: the platform calls again
    let mut failing = change.clone();
    failing["turn"] = json!(9);
    let resp = service
        .dispatch(Arc::new(Link::detached()), system(failing.to_string()))
        .await;
    assert_eq!(resp.status, 500);
    // no run id: invalid
    let resp = service
        .dispatch(Arc::new(Link::detached()), system("{}".into()))
        .await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn caller_and_stubs() {
    let ctx = Ctx::test("echo", "1.0.0").with_caller(
        Caller::test_user("u-1")
            .with_roles(["echo.clerk", "workspace.admin"])
            .with_scopes(["echo:read"]),
    );
    assert!(ctx.caller().has(&Role::new("clerk")));
    assert!(!ctx.caller().has(&Role::new("manager")));
    assert!(ctx.caller().has(&Role::WORKSPACE_ADMIN));
    assert!(ctx.caller().has_scope(&Scope::READ));
    assert!(!ctx.caller().has_scope(&Scope::WRITE));
    assert_eq!(ctx.caller().kind(), CallerKind::User);
    assert_eq!(ctx.plugin().id(), "echo");
    let err = ctx.events().emit(CountRequest {}).await.unwrap_err();
    assert_eq!(
        (err.status(), err.message()),
        (503, "events are not available")
    );
    let err = ctx.transaction(|_| async { Ok(1) }).await.unwrap_err();
    assert_eq!(err.status(), 503);
}

// --- the runs of the plugin's agents ------------------------------------------------------------------

fn wire_run(id: &str, status: &str) -> pb::AgentRun {
    pb::AgentRun {
        id: id.into(),
        agent: "assistant".into(),
        plugin: "echo".into(),
        engine: "claude".into(),
        status: status.into(),
        input: "what does tea cost?".into(),
        result: if status == "succeeded" {
            "Tea is $4.".into()
        } else {
            String::new()
        },
        started_by: Some(pb::AgentRunActor {
            user_id: "u-42".into(),
            display_name: "Sarah Bennett".into(),
        }),
        started_by_plugin: "echo".into(),
        created_at: 1_000,
        updated_at: 2_000,
        deadline_at: 1_801_000,
        turn: 1,
        last_seq: 3,
        pod: "running".into(),
        ..Default::default()
    }
}

/// A context of request 77 whose syscalls a fake platform answers; the calls it saw.
fn ctx_with_a_cell(
    answer: impl Fn(&syscall::Call) -> std::result::Result<syscall_result::Result, String>
    + Send
    + 'static,
) -> (Ctx, Arc<std::sync::Mutex<Vec<syscall::Call>>>) {
    let (tx, mut rx) = mpsc::channel(16);
    let link = Arc::new(Link::new(tx));
    let seen: Arc<std::sync::Mutex<Vec<syscall::Call>>> = Arc::default();
    {
        let (link, seen) = (Arc::downgrade(&link), seen.clone());
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let Some(plugin_message::Kind::Syscall(sc)) = msg.kind else {
                    continue;
                };
                let call = sc.call.unwrap();
                let (result, error) = match answer(&call) {
                    Ok(r) => (Some(r), String::new()),
                    Err(e) => (None, e),
                };
                seen.lock().unwrap().push(call);
                let Some(link) = link.upgrade() else { break };
                link.complete(pb::SyscallResult {
                    id: sc.id,
                    error,
                    result,
                    error_details: None,
                });
            }
        });
    }
    let ctx = Ctx::new(
        link,
        Arc::new(PluginInfo::new("echo", "1.0.0")),
        RequestInfo::new(77, Duration::from_secs(5), None),
        Caller::from_principal(None, "echo"),
    );
    (ctx, seen)
}

#[tokio::test]
async fn agents_run_and_follow() {
    let (ctx, seen) = ctx_with_a_cell(|call| {
        use syscall_result::Result as R;
        Ok(match call {
            syscall::Call::AgentRun(_) => R::AgentRun(wire_run("run-1", "starting")),
            syscall::Call::AgentGet(g) => R::AgentRun(wire_run(&g.run_id, "succeeded")),
            syscall::Call::AgentCancel(c) => R::AgentRun(wire_run(&c.run_id, "cancelled")),
            syscall::Call::AgentClose(c) => R::AgentRun(wire_run(&c.run_id, "idle")),
            syscall::Call::AgentSend(_) => R::AgentInput(pb::AgentRunInput { seq: 2 }),
            syscall::Call::AgentRuns(_) => R::AgentRuns(pb::AgentRunsPage {
                runs: vec![wire_run("run-2", "running"), wire_run("run-1", "succeeded")],
            }),
            syscall::Call::AgentEvents(e) => R::AgentEvents(pb::AgentRunEventsPage {
                run: Some(wire_run(&e.run_id, "running")),
                events: vec![pb::AgentRunEvent {
                    seq: 3,
                    at: 1_500,
                    r#type: "tool".into(),
                    json: br#"{"seq":3,"at":1500,"type":"tool","name":"shell","phase":"completed","summary":"celobus whoami"}"#
                        .to_vec()
                        .into(),
                }],
                partial: Some(pb::AgentRunPartial {
                    rev: 4,
                    messages: vec![
                        pb::AgentRunPartialMessage {
                            message_id: "m1".into(),
                            text: "Tea".into(),
                        },
                        pb::AgentRunPartialMessage {
                            message_id: "m2".into(),
                            text: "is $4".into(),
                        },
                    ],
                }),
            }),
            other => panic!("unexpected syscall {other:?}"),
        })
    });
    let agents = ctx.agents();

    // a run is started in the request being handled: its caller is who starts it
    let run = agents
        .run("assistant", "what does tea cost?")
        .await
        .unwrap();
    assert_eq!(
        (run.id.as_str(), run.status),
        ("run-1", AgentRunStatus::Starting)
    );
    assert!(!run.ended());
    assert_eq!(run.started_by.user_id.to_string(), "u-42");
    assert_eq!(run.started_by_plugin.as_deref(), Some("echo"));
    assert_eq!((run.model.as_deref(), run.result.as_deref()), (None, None));
    let options = RunOptions::default()
        .engine(AgentEngine::Pi)
        .model("claude-opus-5-5")
        .session(true)
        .timeout(Duration::from_secs(600))
        .notify();
    agents
        .run_with("assistant", "and coffee?", options)
        .await
        .unwrap();

    let done = agents.get("run-1").await.unwrap();
    assert_eq!(done.status, AgentRunStatus::Succeeded);
    assert!(done.ended());
    assert_eq!(done.result.as_deref(), Some("Tea is $4."));

    // events keep the fields of their type; the unfinished text comes with them
    let page = agents.events("run-1", 2, 50).await.unwrap();
    assert_eq!(page.run.last_seq, 3);
    let tool = &page.events[0];
    assert_eq!((tool.seq, tool.at, tool.kind.as_str()), (3, 1_500, "tool"));
    assert_eq!(
        (
            tool.str("name"),
            tool.str("phase"),
            tool.str("summary"),
            tool.text()
        ),
        (
            Some("shell"),
            Some("completed"),
            Some("celobus whoami"),
            None
        )
    );
    assert_eq!(tool.data["seq"], json!(3));
    assert_eq!(page.partial.rev, 4);
    assert_eq!(page.partial.text(), "Tea\n\nis $4");

    let runs = agents.list(Some("assistant"), 10).await.unwrap();
    assert_eq!(
        runs.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["run-2", "run-1"]
    );
    agents.list(None, 0).await.unwrap();
    assert_eq!(agents.send("run-1", "and coffee?").await.unwrap(), 2);
    assert_eq!(
        agents.cancel("run-1").await.unwrap().status,
        AgentRunStatus::Cancelled
    );
    assert_eq!(
        agents.close("run-1").await.unwrap().status,
        AgentRunStatus::Idle
    );

    // what went to the platform: the request's id with what acts for its caller
    let seen = seen.lock().unwrap();
    let syscall::Call::AgentRun(start) = &seen[0] else {
        panic!("not a start");
    };
    assert_eq!(
        (start.agent.as_str(), start.input.as_str(), start.request_id),
        ("assistant", "what does tea cost?", 77)
    );
    assert_eq!(
        (
            start.engine.as_str(),
            start.model.as_str(),
            start.session,
            start.timeout_s
        ),
        ("", "", None, 0)
    );
    assert!(!start.notify);
    let syscall::Call::AgentRun(start) = &seen[1] else {
        panic!("not a start");
    };
    assert_eq!(
        (
            start.engine.as_str(),
            start.model.as_str(),
            start.session,
            start.timeout_s
        ),
        ("pi", "claude-opus-5-5", Some(true), 600)
    );
    assert!(start.notify && start.notify_link.is_none());
    let syscall::Call::AgentEvents(e) = &seen[3] else {
        panic!("not events");
    };
    assert_eq!((e.run_id.as_str(), e.after, e.limit), ("run-1", 2, 50));
    let syscall::Call::AgentRuns(l) = &seen[4] else {
        panic!("not a list");
    };
    assert_eq!((l.agent.as_str(), l.limit), ("assistant", 10));
    let syscall::Call::AgentRuns(l) = &seen[5] else {
        panic!("not a list");
    };
    assert_eq!((l.agent.as_str(), l.limit), ("", 0));
    let syscall::Call::AgentSend(s) = &seen[6] else {
        panic!("not a send");
    };
    assert_eq!(
        (s.run_id.as_str(), s.text.as_str(), s.request_id),
        ("run-1", "and coffee?", 77)
    );
}

#[tokio::test]
async fn agents_work_in_groups() {
    let (ctx, seen) = ctx_with_a_cell(|call| match call {
        // a run of the group run-1 that the agent run run-1 brought in
        syscall::Call::AgentRun(r) if !r.group.is_empty() => {
            Ok(syscall_result::Result::AgentRun(pb::AgentRun {
                group: r.group.clone(),
                parent: "run-1".into(),
                parent_agent: "echo.assistant".into(),
                on_behalf_of: Some(pb::AgentRunActor {
                    user_id: "u-7".into(),
                    display_name: "Marcus Lee".into(),
                }),
                ..wire_run("run-2", "starting")
            }))
        }
        // a platform that names no group and nobody the run works for
        syscall::Call::AgentRun(_) => Ok(syscall_result::Result::AgentRun(wire_run(
            "run-1", "starting",
        ))),
        syscall::Call::AgentRuns(_) => Ok(syscall_result::Result::AgentRuns(pb::AgentRunsPage {
            runs: vec![],
        })),
        _ => Err("unknown_group: the platform remembers no group".to_string()),
    });
    let agents = ctx.agents();
    // a run started without a group founds one, named by its id; it works for who started it
    let first = agents.run("assistant", "x").await.unwrap();
    assert_eq!(first.group, "run-1");
    assert_eq!(
        (first.parent.as_deref(), first.parent_agent.as_deref()),
        (None, None)
    );
    assert_eq!(first.on_behalf_of, first.started_by);
    let options = RunOptions::default().group(&first.group);
    let second = agents.run_with("assistant", "y", options).await.unwrap();
    assert_eq!(second.group, "run-1");
    assert_eq!(
        (second.parent.as_deref(), second.parent_agent.as_deref()),
        (Some("run-1"), Some("echo.assistant"))
    );
    assert_eq!(second.on_behalf_of.display_name, "Marcus Lee");
    assert_eq!(second.started_by.display_name, "Sarah Bennett");
    assert!(agents.group("run-1").await.unwrap().is_empty());
    let e = agents.get("run-1").await.unwrap_err();
    assert_eq!((e.status(), e.code()), (404, "unknown_group"));

    let seen = seen.lock().unwrap();
    let groups: Vec<&str> = seen
        .iter()
        .filter_map(|c| match c {
            syscall::Call::AgentRun(r) => Some(r.group.as_str()),
            syscall::Call::AgentRuns(l) => Some(l.group.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(groups, ["", "run-1", "run-1"]);
}

#[tokio::test]
async fn agents_of_other_plugins_are_listed_and_run_on_a_topic_of_the_plugin() {
    #[derive(crate::Message, Clone, Debug)]
    pub struct Conversation {
        pub id: String,
    }
    const CONVERSATION: crate::ui::Topic<Conversation> = crate::ui::Topic::new("conversation");
    let (ctx, seen) = ctx_with_a_cell(|call| {
        use syscall_result::Result as R;
        Ok(match call {
            syscall::Call::AgentsAvailable(_) => R::AgentsAvailable(pb::AvailableAgents {
                agents: vec![
                    pb::AvailableAgent {
                        agent: "echo.assistant".into(),
                        plugin: "echo".into(),
                        name: "assistant".into(),
                        title: "assistant".into(),
                        plugin_title: "Echo".into(),
                        ..Default::default()
                    },
                    pb::AvailableAgent {
                        agent: "crm.assistant".into(),
                        plugin: "crm".into(),
                        name: "assistant".into(),
                        title: "CRM assistant".into(),
                        description: "Answers about deals.".into(),
                        icon: "wand-sparkles".into(),
                        engine: "claude".into(),
                        session: true,
                        plugin_title: "CRM".into(),
                    },
                ],
            }),
            syscall::Call::AgentRun(_) => R::AgentRun(pb::AgentRun {
                agent: "assistant".into(),
                plugin: "crm".into(),
                ..wire_run("run-1", "starting")
            }),
            other => panic!("unexpected syscall {other:?}"),
        })
    });
    let agents = ctx.agents();
    let available = agents.available().await.unwrap();
    assert_eq!(
        available
            .iter()
            .map(|a| a.agent.as_str())
            .collect::<Vec<_>>(),
        ["echo.assistant", "crm.assistant"]
    );
    assert_eq!(
        (available[0].icon.as_deref(), available[0].engine.as_deref()),
        (None, None)
    );
    let creator = &available[1];
    assert_eq!(
        (
            creator.title.as_str(),
            creator.icon.as_deref(),
            creator.engine.as_deref(),
            creator.session,
            creator.plugin_title.as_str()
        ),
        (
            "CRM assistant",
            Some("wand-sparkles"),
            Some("claude"),
            true,
            "CRM"
        )
    );
    let run = agents
        .run_with(
            &creator.agent,
            "make a plugin",
            RunOptions::default().topic_key(CONVERSATION, &Conversation { id: "c-7".into() }),
        )
        .await
        .unwrap();
    assert_eq!(
        (run.plugin.as_str(), run.agent.as_str()),
        ("crm", "assistant")
    );
    let seen = seen.lock().unwrap();
    let syscall::Call::AgentsAvailable(list) = &seen[0] else {
        panic!("not a listing");
    };
    assert_eq!(list.request_id, 77);
    let syscall::Call::AgentRun(start) = &seen[1] else {
        panic!("not a start");
    };
    assert_eq!(
        (
            start.agent.as_str(),
            start.topic.as_str(),
            start.topic_key.as_str()
        ),
        ("crm.assistant", "conversation", r#"{"id":"c-7"}"#)
    );
}

#[tokio::test]
async fn agent_refusals_are_errors_with_the_platforms_code() {
    let (ctx, _) = ctx_with_a_cell(|call| {
        Err(match call {
            syscall::Call::AgentRun(r) if r.agent == "nobody" => {
                "unknown_agent: the plugin echo declares no agent nobody"
            }
            syscall::Call::AgentRun(r) if r.input.is_empty() => "bad_request: input is empty",
            syscall::Call::AgentRun(_) => {
                "forbidden: an agent run is started while the plugin handles a request"
            }
            syscall::Call::AgentGet(_) => "not_found: no such run",
            syscall::Call::AgentSend(_) => "run_ended: the run has ended",
            _ => "store: the store is away",
        }
        .to_string())
    });
    let agents = ctx.agents();
    let code = |e: Error| (e.status(), e.code().to_string());
    assert_eq!(
        code(agents.run("nobody", "x").await.unwrap_err()),
        (404, "unknown_agent".into())
    );
    assert_eq!(
        code(agents.run("assistant", "").await.unwrap_err()),
        (400, "bad_request".into())
    );
    assert_eq!(
        code(agents.run("assistant", "x").await.unwrap_err()),
        (403, "forbidden".into())
    );
    let e = agents.get("run-9").await.unwrap_err();
    assert_eq!(
        (e.status(), e.code(), e.message()),
        (404, "not_found", "no such run")
    );
    assert_eq!(
        code(agents.send("run-1", "more").await.unwrap_err()),
        (409, "run_ended".into())
    );
    assert_eq!(
        code(agents.cancel("run-1").await.unwrap_err()),
        (503, "unavailable".into())
    );
    // without a session: unavailable, like every call to the platform
    let detached = Ctx::test("echo", "1.0.0");
    assert_eq!(
        code(detached.agents().list(None, 0).await.unwrap_err()),
        (503, "unavailable".into())
    );
}

// --- a fake platform ---------------------------------------------------------------------------------

type Outbound = mpsc::Sender<std::result::Result<pb::CelldMessage, tonic::Status>>;

struct FakeCelld {
    sessions: mpsc::Sender<(tonic::Streaming<pb::PluginMessage>, Outbound)>,
}

#[tonic::async_trait]
impl pb::celld_server::Celld for FakeCelld {
    type SessionStream = ReceiverStream<std::result::Result<pb::CelldMessage, tonic::Status>>;

    async fn session(
        &self,
        request: tonic::Request<tonic::Streaming<pb::PluginMessage>>,
    ) -> std::result::Result<tonic::Response<Self::SessionStream>, tonic::Status> {
        let (tx, rx) = mpsc::channel(16);
        let _ = self.sessions.send((request.into_inner(), tx)).await;
        Ok(tonic::Response::new(ReceiverStream::new(rx)))
    }
}

fn to_plugin(kind: celld_message::Kind) -> std::result::Result<pb::CelldMessage, tonic::Status> {
    Ok(pb::CelldMessage { kind: Some(kind) })
}

#[tokio::test]
async fn session_with_a_fake_celld() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("celld.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let (sessions_tx, mut sessions) = mpsc::channel(4);
    tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(pb::celld_server::CelldServer::new(FakeCelld {
                sessions: sessions_tx,
            }))
            .serve_with_incoming(UnixListenerStream::new(listener)),
    );

    let service = Arc::new(Service::new(plugin()).unwrap());
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let addr = format!("unix:{}", sock.display());
    let plugin_task = tokio::spawn(async move {
        serve_until(service, &addr, async {
            let _ = stop_rx.await;
        })
        .await
    });

    let (mut inbound, outbound) = tokio::time::timeout(Duration::from_secs(5), sessions.recv())
        .await
        .unwrap()
        .unwrap();
    let first = inbound.next().await.unwrap().unwrap();
    let Some(plugin_message::Kind::Register(reg)) = first.kind else {
        panic!("the first message is Register: {first:?}");
    };
    assert_eq!(
        (
            reg.plugin.as_str(),
            reg.version.as_str(),
            reg.runtime.as_str()
        ),
        ("echo", "1.0.0", "rust")
    );
    assert_eq!(reg.depends, vec!["greeter".to_string()]);
    assert_eq!(reg.manifest_digest, "");
    assert_eq!(reg.job_slots, 0, "no jobs, no job pool");
    outbound
        .send(to_plugin(celld_message::Kind::Registered(pb::Registered {
            instance_id: "i-1".into(),
        })))
        .await
        .unwrap();

    // a request whose handler makes three syscalls: db_count, db_create, invoke
    outbound
        .send(to_plugin(celld_message::Kind::Request(request(
            77,
            "POST",
            "/echo.v1.Echo/Count",
            Some("application/json"),
            "{}",
        ))))
        .await
        .unwrap();
    let mut stored = None;
    let response = loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), inbound.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match msg.kind {
            Some(plugin_message::Kind::Syscall(sc)) => {
                let result = match sc.call.unwrap() {
                    syscall::Call::DbCount(c) => {
                        assert_eq!((c.entity.as_str(), c.request_id), ("tally", 77));
                        syscall_result::Result::DbCount(pb::DbCountResult {
                            count: 41,
                            ..Default::default()
                        })
                    }
                    syscall::Call::DbCreate(c) => {
                        let row: Value = serde_json::from_slice(&c.rows[0]).unwrap();
                        stored = Some((c.entity, row["n"].clone()));
                        syscall_result::Result::DbRows(pb::DbRows {
                            rows: vec![bytes::Bytes::from(
                                json!({ "id": "1", "n": "42", "createdAt": "2026-11-12T10:00:00Z",
                                        "updatedAt": "2026-11-12T10:00:00Z" })
                                .to_string(),
                            )],
                            next_page_token: String::new(),
                            ..Default::default()
                        })
                    }
                    syscall::Call::Invoke(inv) => {
                        assert_eq!(inv.plugin, "greeter");
                        assert_eq!(inv.request_id, 77);
                        let req = inv.request.unwrap();
                        assert_eq!(
                            (req.method.as_str(), req.path.as_str()),
                            ("POST", "/greeter.v1.Greeter/Greet")
                        );
                        assert_eq!(
                            serde_json::from_slice::<Value>(&req.body).unwrap(),
                            json!({"name": "Sarah"})
                        );
                        syscall_result::Result::InvokeResponse(pb::Response {
                            id: 0,
                            status: 200,
                            headers: vec![],
                            body: br#"{"text":"Hi, Sarah"}"#.to_vec().into(),
                        })
                    }
                    other => panic!("unexpected syscall {other:?}"),
                };
                outbound
                    .send(to_plugin(celld_message::Kind::SyscallResult(
                        pb::SyscallResult {
                            id: sc.id,
                            error: String::new(),
                            result: Some(result),
                            error_details: None,
                        },
                    )))
                    .await
                    .unwrap();
            }
            Some(plugin_message::Kind::Response(resp)) => break resp,
            other => panic!("unexpected message {other:?}"),
        }
    };
    assert_eq!(response.id, 77);
    assert_eq!(response.status, 200);
    assert_eq!(
        body(&response),
        json!({"count": "42", "greeting": "Hi, Sarah"})
    );
    assert_eq!(stored, Some(("tally".to_string(), json!("42"))));

    // a dependency's error comes back as it answered
    outbound
        .send(to_plugin(celld_message::Kind::Request(request(
            78,
            "POST",
            "/echo.v1.Echo/Count",
            None,
            "{}",
        ))))
        .await
        .unwrap();
    let response = loop {
        let msg = inbound.next().await.unwrap().unwrap();
        match msg.kind {
            Some(plugin_message::Kind::Syscall(sc)) => {
                let result = match sc.call.unwrap() {
                    syscall::Call::DbCount(_) => {
                        syscall_result::Result::DbCount(pb::DbCountResult {
                            count: 0,
                            ..Default::default()
                        })
                    }
                    syscall::Call::DbCreate(_) => syscall_result::Result::DbRows(pb::DbRows {
                        rows: vec![bytes::Bytes::from(
                            r#"{"id":"1","n":"1","createdAt":"2026-11-12T10:00:00Z","updatedAt":"2026-11-12T10:00:00Z"}"#,
                        )],
                        next_page_token: String::new(),
                        ..Default::default()
                    }),
                    syscall::Call::Invoke(_) => {
                        syscall_result::Result::InvokeResponse(pb::Response {
                            id: 0,
                            status: 409,
                            headers: vec![],
                            body: br#"{"code":"busy","message":"try later","details":{"n":1}}"#
                                .to_vec()
                                .into(),
                        })
                    }
                    other => panic!("unexpected syscall {other:?}"),
                };
                outbound
                    .send(to_plugin(celld_message::Kind::SyscallResult(
                        pb::SyscallResult {
                            id: sc.id,
                            error: String::new(),
                            result: Some(result),
                            error_details: None,
                        },
                    )))
                    .await
                    .unwrap();
            }
            Some(plugin_message::Kind::Response(resp)) => break resp,
            other => panic!("unexpected message {other:?}"),
        }
    };
    assert_eq!(response.status, 409);
    assert_eq!(
        body(&response),
        json!({"code": "busy", "message": "try later", "details": {"n": 1}})
    );

    // the platform drops the session: the plugin reconnects and registers again
    drop(outbound);
    drop(inbound);
    let (mut inbound, _outbound) = tokio::time::timeout(Duration::from_secs(5), sessions.recv())
        .await
        .unwrap()
        .unwrap();
    let again = inbound.next().await.unwrap().unwrap();
    assert!(matches!(
        again.kind,
        Some(plugin_message::Kind::Register(_))
    ));

    // asked to stop with nothing in flight: it returns
    stop_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), plugin_task)
        .await
        .unwrap()
        .unwrap();
}
