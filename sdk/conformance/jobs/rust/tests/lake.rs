//! The lake leg of the SDK conformance: a fake platform runs the plugin `jobsdemo` of one SDK as a process (the Rust
//! one by default; `CONFORMANCE_PLUGIN="node sdk/conformance/jobs/ts/dist/main.js"` or `"dotnet …/Jobsdemo.dll"` for
//! the others), sends it the requests and job runs of `../../lake.json`, answers its lake writes, reads and refreshes,
//! and checks that it sent exactly the chunks, keys, filters, queries and refreshes that file lists and answered what
//! it lists.
//! `CONFORMANCE_PRINT=<file>` writes what the plugin did there instead of comparing (to write the fixture from the Rust
//! SDK).

#[path = "support/tree.rs"]
mod tree;

use std::process::Stdio;
use std::time::Duration;

use celobus_proto as pb;
use pb::{celld_message, plugin_message, syscall, syscall_result};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::{ReceiverStream, UnixListenerStream};

type Outbound = mpsc::Sender<Result<pb::CelldMessage, tonic::Status>>;

struct FakePlatform {
    sessions: mpsc::Sender<(tonic::Streaming<pb::PluginMessage>, Outbound)>,
}

#[tonic::async_trait]
impl pb::celld_server::Celld for FakePlatform {
    type SessionStream = ReceiverStream<Result<pb::CelldMessage, tonic::Status>>;

    async fn session(
        &self,
        request: tonic::Request<tonic::Streaming<pb::PluginMessage>>,
    ) -> Result<tonic::Response<Self::SessionStream>, tonic::Status> {
        let (tx, rx) = mpsc::channel(64);
        let _ = self.sessions.send((request.into_inner(), tx)).await;
        Ok(tonic::Response::new(ReceiverStream::new(rx)))
    }
}

fn to_plugin(kind: celld_message::Kind) -> Result<pb::CelldMessage, tonic::Status> {
    Ok(pb::CelldMessage { kind: Some(kind) })
}

fn member() -> pb::Principal {
    pb::Principal {
        user_id: "u-42".into(),
        kind: pb::principal::Kind::Person as i32,
        display_name: "Sarah Bennett".into(),
        roles: vec!["workspace.member".into(), "jobsdemo.member".into()],
        groups: vec![],
        scopes: vec!["jobsdemo:read".into(), "jobsdemo:write".into()],
        agent_run: String::new(),
    }
}

/// How the fake answers a lake write's chunk: the write's id is 41; with `last` it commits, unless its key is `seen`
/// (an append written before: skipped) or `busy` (refused as `lake_busy`).
struct Writes {
    rows: u64,
}

impl Writes {
    fn answer(&mut self, w: &pb::LakeWrite) -> (Option<syscall_result::Result>, String) {
        if w.write == 0 {
            self.rows = 0;
            if w.key == "partitions" {
                return (
                    None,
                    "lake_partitions_limit: too many partitions in one write: 1200, at most 1000: write fewer \
                     loaded_on at a time"
                        .into(),
                );
            }
            if w.key == "busy" {
                return (
                    None,
                    "lake_busy: other writes of this table ran for more than 60 s; retry".into(),
                );
            }
        }
        self.rows += w.rows.len() as u64;
        let result = syscall_result::Result::LakeWritten(pb::LakeWritten {
            write: 41,
            rows: self.rows,
            committed: w.last && w.key != "seen",
            skipped: w.last && w.key == "seen",
        });
        (Some(result), String::new())
    }
}

/// How the fake answers a refresh: by its table, whole or by partition (`filter` set). A model over lake tables
/// answers no `as_of`; the others refuse it as the platform would, each with its own code.
fn refresh_answer(r: &pb::LakeRefresh) -> (Option<syscall_result::Result>, String) {
    // a refresh of partitions answers the partitions still holding an earlier declaration's rows
    let stale_partitions = if r.filter.is_some() { 2 } else { 0 };
    let done = |rows: u64, as_of: i64| {
        (
            Some(syscall_result::Result::LakeRefreshed(pb::LakeRefreshed {
                rows,
                as_of,
                stale_partitions,
            })),
            String::new(),
        )
    };
    // a filter that names no partition is refused, as the platform refuses it
    if r.filter.as_ref().is_some_and(|f| f.node.is_none()) {
        return (
            None,
            "invalid: replace's filter names the partitions it swaps: compare day".into(),
        );
    }
    match (r.table.as_str(), r.filter.is_some()) {
        ("task_hours", false) => done(25, 0),
        ("task_hours", true) => done(5, 0),
        ("budget_copy", _) => done(3, 1_795_000_000_000),
        ("open_by_day", false) => done(2, 1_795_000_060_000),
        ("issue_week", false) => (
            None,
            "lake_model_rows_limit: the model computes more than 1000000 rows: refresh fewer partitions at once"
                .into(),
        ),
        ("issue_week", true) => (
            None,
            "lake_model_parse: a text of the column issues does not read as an int64".into(),
        ),
        ("open_by_day", true) => (
            None,
            "lake_model_changed: the model of open_by_day changed: refresh the whole table first".into(),
        ),
        _ => (
            None,
            "unavailable: the workspace's analytic copy is loading; retry".into(),
        ),
    }
}

/// How the fake answers a ticket's state, by the ticket: `t-1` is building until it is waited on twice, then done;
/// `rows`, `time`, `dup`, `invalid` and `failed` failed with that load's code, `blank` failed without one; `busy` and
/// `gone` are refused (`analytic_load_busy`, `not_found`).
fn ticket_answer(
    s: &pb::AnalyticsLoadStatus,
    asked: u32,
) -> (Option<syscall_result::Result>, String) {
    use pb::analytics_ticket::State;
    let ticket = |state: State, code: &str| pb::AnalyticsTicket {
        ticket: s.ticket.clone(),
        state: state as i32,
        entities: vec!["stage_day".into()],
        requested_at: 1_795_000_000_000,
        code: code.into(),
        error: if code.is_empty() && state != State::Failed {
            String::new()
        } else {
            "the load of stage_day failed".into()
        },
        ..Default::default()
    };
    let failed = |code: &str| {
        (
            Some(syscall_result::Result::AnalyticsTicket(ticket(
                State::Failed,
                code,
            ))),
            String::new(),
        )
    };
    match s.ticket.as_str() {
        "t-1" if asked < 2 => (
            Some(syscall_result::Result::AnalyticsTicket(ticket(
                State::Building,
                "",
            ))),
            String::new(),
        ),
        "t-1" => {
            let mut t = ticket(State::Done, "");
            t.generation = 7;
            t.finished_at = 1_795_000_600_000;
            t.loads = vec![pb::AnalyticsLoadOutcome {
                entity: "stage_day".into(),
                rows: 4,
                loaded_at: 1_795_000_500_000,
                ..Default::default()
            }];
            (
                Some(syscall_result::Result::AnalyticsTicket(t)),
                String::new(),
            )
        }
        "rows" => failed("analytic_load_rows_limit"),
        "time" => failed("analytic_load_time_limit"),
        "dup" => failed("analytic_load_duplicate"),
        "invalid" => failed("analytic_load_invalid"),
        "failed" => failed("analytic_load_failed"),
        "blank" => failed(""),
        "busy" => (
            None,
            "analytic_load_busy: the plugin waits on 16 tickets already; retry".into(),
        ),
        _ => (None, "not_found: no such ticket of the plugin".into()),
    }
}

/// A refresh as the fixture lists it: its table and, by partition, its filter.
fn refresh_json(r: &pb::LakeRefresh) -> Value {
    let mut v = json!({ "table": r.table });
    if let Some(f) = &r.filter {
        // set but naming nothing: `{}`
        v["filter"] = if f.node.is_none() {
            json!({})
        } else {
            tree::filter_json(f)
        };
    }
    v
}

/// The rows the fake answers a query with: a history's rows as objects (two changes of a task, or per day two days of
/// one version), any other query one row of values.
fn query_rows(q: &pb::Query) -> Vec<bytes::Bytes> {
    let root = q.sources.first();
    if !root.is_some_and(|r| r.history) || !q.select.is_empty() {
        return vec![bytes::Bytes::from(r#"["2026-11-01","s1","3","4.5"]"#)];
    }
    let each_day = root.is_some_and(|r| r.each_day);
    let row = |change: &str, status: Option<&str>, to: Option<&str>, day: &str| {
        let mut v = json!({
            "id": "0190f3c2-0000-7000-8000-000000000101",
            "validFrom": "2026-11-19T10:00:00Z",
            "validTo": to,
            "change": change,
            "changedBy": "0190f3c2-0000-7000-8000-0000000000a1",
            "changedByKind": "person",
            "changedByPlugin": "jobsdemo",
        });
        if let Some(s) = status {
            v["status"] = json!(s);
        }
        if each_day {
            v["day"] = json!(day);
        }
        bytes::Bytes::from(v.to_string())
    };
    vec![
        row(
            "insert",
            Some("new"),
            Some("2026-11-21T08:30:00Z"),
            "2026-11-19",
        ),
        row("update", None, None, "2026-11-20"),
    ]
}

/// A chunk as the fixture lists it: the call's members, its rows counted, its first and last row (a text longer than
/// 64 bytes as its length).
fn chunk_json(w: &pb::LakeWrite) -> Value {
    let row = |b: Option<&bytes::Bytes>| {
        let Some(b) = b else { return Value::Null };
        let mut v: Value = serde_json::from_slice(b).expect("a row is JSON");
        for f in v.as_object_mut().expect("a row is an object").values_mut() {
            if let Value::String(s) = f
                && s.len() > 64
            {
                *f = Value::String(format!("<{} bytes>", s.len()));
            }
        }
        v
    };
    let mode = pb::lake_write::Mode::try_from(w.mode).unwrap();
    let mut v = json!({
        "table": w.table,
        "mode": mode.as_str_name(),
        "key": w.key,
        "write": w.write.to_string(),
        "last": w.last,
        "rows": w.rows.len(),
        "first": row(w.rows.first()),
        "lastRow": row(w.rows.last()),
    });
    if let Some(f) = &w.filter {
        v["filter"] = tree::filter_json(f);
    }
    v
}

/// One step of the fixture run against the plugin: what it answered, the chunks and queries it sent.
async fn run_step(
    step: &Value,
    inbound: &mut tonic::Streaming<pb::PluginMessage>,
    outbound: &Outbound,
    id: u64,
) -> Value {
    let job = step.get("job").is_some();
    if job {
        let payload = step["payload"].to_string();
        outbound
            .send(to_plugin(celld_message::Kind::JobRun(pb::JobRun {
                run_id: step["run"].as_str().unwrap().into(),
                job: step["job"].as_str().unwrap().into(),
                attempt: step["attempt"].as_u64().unwrap() as u32,
                max_attempts: 3,
                payload,
                principal: Some(member()),
                scheduled_at_unix_ms: 1_795_000_000_000,
                timeout_ms: 120_000,
                request_id: id,
            })))
            .await
            .unwrap();
    } else {
        let path = format!(
            "/jobsdemo.v1.Jobsdemo/{}",
            step["endpoint"].as_str().unwrap()
        );
        outbound
            .send(to_plugin(celld_message::Kind::Request(pb::Request {
                id,
                method: "POST".into(),
                path,
                headers: vec![pb::Header {
                    name: "Content-Type".into(),
                    value: "application/json".into(),
                }],
                body: step["body"].to_string().into_bytes().into(),
                principal: Some(member()),
                ..Default::default()
            })))
            .await
            .unwrap();
    }
    let mut writes = Writes { rows: 0 };
    let mut chunks = vec![];
    let mut queries = vec![];
    let mut refreshes = vec![];
    let mut loads = vec![];
    let mut asked = 0u32;
    // what the step asks the fake to refuse its requests for loads and its queries with
    let refuse = step["refuse"].as_str().map(str::to_string);
    let answer = loop {
        let msg = tokio::time::timeout(Duration::from_secs(120), inbound.next())
            .await
            .expect("the plugin answers within two minutes")
            .expect("the session is open")
            .expect("a message");
        match msg.kind {
            Some(plugin_message::Kind::Syscall(sc)) => {
                let (result, error) = match sc.call.unwrap() {
                    syscall::Call::LakeWrite(w) => {
                        assert_eq!(w.request_id, id, "a lake write carries the request's id");
                        chunks.push(chunk_json(&w));
                        writes.answer(&w)
                    }
                    syscall::Call::Query(q) if refuse.is_some() => {
                        let engine = pb::Engine::try_from(q.engine).unwrap();
                        let mode = pb::query_read::Mode::try_from(q.mode).unwrap();
                        queries.push(json!({
                            "engine": engine.as_str_name(),
                            "mode": mode.as_str_name(),
                            "query": tree::tree_json(q.query.as_ref().unwrap()),
                        }));
                        (
                            None,
                            format!("{}: refused by the fake", refuse.as_deref().unwrap()),
                        )
                    }
                    syscall::Call::AnalyticsLoad(r) => {
                        assert_eq!(
                            r.request_id, id,
                            "a request for loads carries the request's id"
                        );
                        loads.push(json!({ "request": r.entities }));
                        match &refuse {
                            Some(code) => (None, format!("{code}: refused by the fake")),
                            None => (
                                Some(syscall_result::Result::AnalyticsTicket(
                                    pb::AnalyticsTicket {
                                        ticket: "t-1".into(),
                                        entities: r.entities.clone(),
                                        requested_at: 1_795_000_000_000,
                                        ..Default::default()
                                    },
                                )),
                                String::new(),
                            ),
                        }
                    }
                    syscall::Call::AnalyticsLoadStatus(s) => {
                        assert_eq!(
                            s.request_id, id,
                            "a ticket's state carries the request's id"
                        );
                        loads.push(json!({ "ticket": s.ticket, "waitMs": s.wait_ms }));
                        asked += 1;
                        ticket_answer(&s, asked)
                    }
                    syscall::Call::Query(q) => {
                        let engine = pb::Engine::try_from(q.engine).unwrap();
                        let mode = pb::query_read::Mode::try_from(q.mode).unwrap();
                        queries.push(json!({
                            "engine": engine.as_str_name(),
                            "mode": mode.as_str_name(),
                            "query": tree::tree_json(q.query.as_ref().unwrap()),
                        }));
                        let rows = query_rows(q.query.as_ref().unwrap());
                        (
                            Some(syscall_result::Result::DbRows(pb::DbRows {
                                rows,
                                ..Default::default()
                            })),
                            String::new(),
                        )
                    }
                    syscall::Call::LakeRefresh(r) => {
                        assert_eq!(r.request_id, id, "a refresh carries the request's id");
                        refreshes.push(refresh_json(&r));
                        refresh_answer(&r)
                    }
                    other => (None, format!("unavailable: not in this fake ({other:?})")),
                };
                outbound
                    .send(to_plugin(celld_message::Kind::SyscallResult(
                        pb::SyscallResult {
                            id: sc.id,
                            error,
                            result,
                            error_details: None,
                        },
                    )))
                    .await
                    .unwrap();
            }
            Some(plugin_message::Kind::Response(r)) if !job => {
                assert_eq!(r.id, id);
                let body: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
                break if r.status == 200 {
                    json!({ "status": 200, "body": body })
                } else {
                    json!({ "status": r.status, "code": body["code"] })
                };
            }
            Some(plugin_message::Kind::JobDone(d)) if job => {
                break match d.outcome {
                    Some(pb::job_done::Outcome::Ok(_)) => json!({ "done": "ok" }),
                    Some(pb::job_done::Outcome::Error(e)) => json!({ "done": e.code }),
                    None => json!({ "done": null }),
                };
            }
            // log lines of a run
            _ => {}
        }
    };
    let mut out = json!({ "answer": answer, "writes": chunks });
    if !queries.is_empty() {
        out["queries"] = Value::Array(queries);
    }
    if !refreshes.is_empty() {
        out["refreshes"] = Value::Array(refreshes);
    }
    if !loads.is_empty() {
        out["loads"] = Value::Array(loads);
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn the_plugin_sends_the_conformance_lake_calls() {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(here.join("../lake.json")).unwrap()).unwrap();
    let lake_query: Value =
        serde_json::from_str(&std::fs::read_to_string(here.join("../../query/lake.json")).unwrap())
            .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("platform.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let (sessions_tx, mut sessions) = mpsc::channel(4);
    tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(
                pb::celld_server::CelldServer::new(FakePlatform {
                    sessions: sessions_tx,
                })
                .max_decoding_message_size(64 << 20),
            )
            .serve_with_incoming(UnixListenerStream::new(listener)),
    );

    let command = std::env::var("CONFORMANCE_PLUGIN")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_jobsdemo").to_string());
    let mut words = command.split_whitespace();
    let mut plugin = tokio::process::Command::new(words.next().unwrap());
    plugin
        .args(words)
        .env("CELOBUS_ADDR", format!("unix:{}", sock.display()))
        .env("CELOBUS_LOG", "warn")
        .stdout(Stdio::null())
        .kill_on_drop(true);
    let proto = here.join("../../../../proto/plugin.proto");
    if proto.exists() {
        plugin.env("CELOBUS_PROTO", proto);
    }
    let mut child = plugin.spawn().expect("the plugin starts");

    let (mut inbound, outbound) = tokio::time::timeout(Duration::from_secs(60), sessions.recv())
        .await
        .expect("the plugin connects within a minute")
        .unwrap();
    let first = inbound.next().await.unwrap().unwrap();
    assert!(
        matches!(first.kind, Some(plugin_message::Kind::Register(_))),
        "the first message is Register"
    );
    outbound
        .send(to_plugin(celld_message::Kind::Registered(pb::Registered {
            instance_id: "i-1".into(),
            ..Default::default()
        })))
        .await
        .unwrap();

    let print = std::env::var_os("CONFORMANCE_PRINT");
    let mut failed = vec![];
    let mut printed = vec![];
    for (i, step) in fixture["steps"].as_array().unwrap().iter().enumerate() {
        let got = run_step(step, &mut inbound, &outbound, 100 + i as u64).await;
        if print.is_some() {
            let mut s = step.clone();
            s["expect"] = got;
            printed.push(s);
            continue;
        }
        let mut expected = step["expect"].clone();
        if let Some(qs) = expected.get_mut("queries").and_then(Value::as_array_mut) {
            for q in qs {
                // the query of `TaskDays` is the shared tree
                if q["query"] == "query/lake.json" {
                    q["query"] = lake_query.clone();
                }
            }
        }
        if got != expected {
            failed.push(format!(
                "step {i} ({}):\n  expected {}\n  got      {}",
                step["name"].as_str().unwrap_or(""),
                expected,
                got
            ));
        }
    }
    let _ = child.start_kill();
    if let Some(path) = print {
        let out = serde_json::to_string_pretty(&json!({ "steps": printed })).unwrap();
        std::fs::write(path, out).unwrap();
        return;
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
