//! Jobs: the declaration and its descriptor, enqueue and cancel on the wire, and runs from `JobRun` to `JobDone`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use celobus_proto as pb;
use pb::{plugin_message, syscall, syscall_result};
use serde_json::json;
use tokio::sync::{Semaphore, mpsc};

use crate::ctx::{Caller, Link, RequestInfo};
use crate::jobs::{Enqueued, RunState};
use crate::prelude::*;
use crate::session::Service;

/// Remind the clerks of unpaid orders.
#[derive(Message, Clone, Debug, Default)]
#[message(job)]
pub struct RemindUnpaid {
    pub older_than_days: i32,
}

#[derive(Message, Clone, Debug, Default)]
#[message(job)]
pub struct Reindex {
    pub how: String,
}

#[derive(Message, Clone, Debug)]
pub struct NotAJob {}

async fn remind(ctx: Ctx, p: RemindUnpaid) -> Result<()> {
    tracing::info!(days = p.older_than_days, "reminding");
    assert!(ctx.job().is_run());
    assert_eq!(ctx.request().id(), 900);
    Ok(())
}

async fn reindex(ctx: Ctx, p: Reindex) -> Result<()> {
    match p.how.as_str() {
        "permanent" => Err(Error::permanent("customer has no e-mail")),
        "invalid" => Err(Error::invalid("bad")),
        "busy" => Err(Error::unavailable("the mail server is busy")),
        "panic" => panic!("boom"),
        "hang" => {
            tracing::info!("hanging");
            std::future::pending::<()>().await;
            Ok(())
        }
        "info" => {
            let j = ctx.job();
            assert_eq!(
                (j.job(), j.run_id(), j.attempt(), j.max_attempts()),
                ("reindex", "run-1", 2, 4)
            );
            assert_eq!(j.scheduled_at().timestamp_millis(), 1_800_000_000_000);
            assert!(!j.is_last_attempt());
            assert_eq!(ctx.caller().user()?.to_string(), "u-7");
            Ok(())
        }
        _ => Ok(()),
    }
}

fn plugin() -> Plugin {
    Plugin::new("orders", "1.0.0")
        .job(
            Job::new(remind)
                .describe("Remind unpaid orders")
                .cron("0 9 * * 1-5")
                .as_system()
                .retry(Retry::exponential(3, Duration::from_secs(60)))
                .timeout(Duration::from_secs(120))
                .concurrency(2),
        )
        .job(Job::new(reindex))
}

#[test]
fn a_job_declares_its_descriptor_entry_and_payload() {
    let d = plugin().descriptor().unwrap();
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(
        v["jobs"],
        json!([
            {"name": "remind_unpaid", "payload": "RemindUnpaid", "description": "Remind unpaid orders",
             "cron": "0 9 * * 1-5", "as_system": true,
             "retry": {"retries": 3, "delay_ms": 60000, "backoff": "exponential"},
             "timeout_ms": 120000, "concurrency": 2},
            {"name": "reindex", "payload": "Reindex"}
        ])
    );
    let m = d
        .messages
        .iter()
        .find(|m| m.name == "RemindUnpaid")
        .unwrap();
    assert_eq!(m.kind, "job");
    // the descriptor crate reads it back as it wrote it
    let checked = celobus_descriptor::check(d.to_canonical_json().as_bytes()).unwrap();
    assert_eq!(
        checked.descriptor.jobs[0].retry_or_default().delay_ms,
        60_000
    );
    assert_eq!(checked.descriptor.jobs[1].timeout_ms_or_default(), 300_000);
    // no jobs: no `jobs` key
    let v = serde_json::to_value(Plugin::new("orders", "1.0.0").descriptor().unwrap()).unwrap();
    assert!(v.get("jobs").is_none());
}

#[test]
fn the_jobs_rules_are_checked_at_build() {
    let codes = |p: Plugin| -> Vec<&'static str> {
        p.descriptor()
            .unwrap_err()
            .0
            .iter()
            .map(|e| e.code)
            .collect()
    };
    let p = Plugin::new("orders", "1.0.0").job(Job::new(remind).cron("61 * * * *"));
    assert_eq!(codes(p), vec!["E_JOB_CRON"]);
    let p = Plugin::new("orders", "1.0.0")
        .job(Job::new(reindex))
        .job(Job::new(reindex).name("reindex_again"));
    assert_eq!(codes(p), vec!["E_JOB_PAYLOAD_DUP"]);
    let p = Plugin::new("orders", "1.0.0").job(
        Job::new(reindex)
            .name("Re-index")
            .retry(Retry::fixed(21, Duration::from_secs(1)))
            .timeout(Duration::from_millis(10))
            .concurrency(65),
    );
    assert_eq!(
        codes(p),
        vec![
            "E_JOB_NAME",
            "E_JOB_RETRY",
            "E_JOB_TIMEOUT",
            "E_JOB_CONCURRENCY"
        ]
    );
    let p = plugin().job_slots(65);
    assert_eq!(codes(p), vec!["E_JOB_SLOTS"]);
}

#[test]
fn the_job_pool_is_announced_only_with_jobs() {
    assert_eq!(Service::new(plugin()).unwrap().job_slots(), 16);
    assert_eq!(Service::new(plugin().job_slots(4)).unwrap().job_slots(), 4);
    assert_eq!(
        Service::new(Plugin::new("orders", "1.0.0"))
            .unwrap()
            .job_slots(),
        0
    );
}

#[test]
fn permanence_is_the_flag_or_an_input_class_error() {
    assert!(Error::permanent("no mail").is_permanent());
    assert_eq!(Error::permanent("no mail").code(), "permanent");
    assert!(Error::internal("x").into_permanent().is_permanent());
    for e in [
        Error::invalid("x"),
        Error::not_found("x"),
        Error::new(403, "permission_denied", "x"),
        Error::new(400, "payload_invalid", "x"),
    ] {
        assert!(e.is_permanent(), "{e}");
    }
    for e in [
        Error::internal("x"),
        Error::unavailable("x"),
        Error::forbidden("x"),
        Error::conflict("x"),
        Error::deadline_exceeded("x"),
    ] {
        assert!(!e.is_permanent(), "{e}");
    }
}

/// A context of request 77 of the plugin above whose syscalls a fake platform answers; the calls it saw.
fn ctx_with_a_platform() -> (Ctx, Arc<Mutex<Vec<syscall::Call>>>) {
    let (tx, mut rx) = mpsc::channel::<pb::PluginMessage>(16);
    let link = Arc::new(Link::new(tx));
    let seen: Arc<Mutex<Vec<syscall::Call>>> = Arc::default();
    {
        let (link, seen) = (Arc::downgrade(&link), seen.clone());
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let Some(plugin_message::Kind::Syscall(sc)) = msg.kind else {
                    continue;
                };
                let call = sc.call.unwrap();
                let (result, error) = match &call {
                    syscall::Call::TxBegin(_) => (
                        Some(syscall_result::Result::TxBegun(pb::TxBegun { tx: 5 })),
                        String::new(),
                    ),
                    syscall::Call::JobEnqueue(e) if e.singleton == "full" => {
                        (None, "queue_full: 10,000 pending runs".to_string())
                    }
                    syscall::Call::JobEnqueue(e) => (
                        Some(syscall_result::Result::JobEnqueued(pb::JobEnqueued {
                            run_id: format!("run-{}", e.job),
                            existing: !e.singleton.is_empty(),
                            state: pb::RunState::Pending as i32,
                        })),
                        String::new(),
                    ),
                    syscall::Call::JobCancel(c) if c.run_id == "nope" => {
                        (None, "not_found: no run nope".to_string())
                    }
                    syscall::Call::JobCancel(_) => (
                        Some(syscall_result::Result::JobCancelled(pb::JobCancelled {
                            cancelled: true,
                        })),
                        String::new(),
                    ),
                    _ => (None, String::new()),
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
    let service = Service::new(plugin()).unwrap();
    let ctx = Ctx::new(
        link,
        service.info(),
        RequestInfo::new(77, Duration::from_secs(5), None),
        Caller::from_principal(None, "orders"),
    );
    (ctx, seen)
}

#[tokio::test]
async fn enqueue_and_cancel_on_the_wire() {
    let (ctx, seen) = ctx_with_a_platform();
    let run = ctx
        .jobs()
        .enqueue(RemindUnpaid { older_than_days: 7 })
        .delay(Duration::from_secs(3600))
        .await
        .unwrap();
    assert_eq!(
        run,
        Enqueued {
            run_id: "run-remind_unpaid".into(),
            existing: false,
            state: RunState::Pending
        }
    );
    let at = chrono::DateTime::from_timestamp_millis(1_900_000_000_000).unwrap();
    let again = ctx
        .jobs()
        .enqueue(Reindex::default())
        .at(at)
        .singleton("report:2026-11-20")
        .await
        .unwrap();
    assert!(again.existing);
    // in a declared transaction the enqueue carries it
    ctx.transaction(|tx| async move {
        tx.jobs().enqueue(Reindex { how: "x".into() }).await?;
        Ok(())
    })
    .await
    .unwrap();
    assert!(ctx.jobs().cancel(&run).await.unwrap());
    let e = ctx.jobs().cancel("nope").await.unwrap_err();
    assert_eq!((e.status(), e.code()), (404, "not_found"));
    let e = ctx
        .jobs()
        .enqueue(Reindex::default())
        .singleton("full")
        .await
        .unwrap_err();
    assert_eq!((e.status(), e.code()), (429, "queue_full"));

    let calls = seen.lock().unwrap().clone();
    let enqueues: Vec<pb::JobEnqueue> = calls
        .iter()
        .filter_map(|c| match c {
            syscall::Call::JobEnqueue(e) => Some(e.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(enqueues[0].job, "remind_unpaid");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&enqueues[0].payload).unwrap(),
        json!({"olderThanDays": 7})
    );
    assert_eq!(
        enqueues[0].when,
        Some(pb::job_enqueue::When::DelayMs(3_600_000))
    );
    assert_eq!((enqueues[0].request_id, enqueues[0].tx), (77, 0));
    assert_eq!(
        enqueues[1].when,
        Some(pb::job_enqueue::When::AtUnixMs(1_900_000_000_000))
    );
    assert_eq!(enqueues[1].singleton, "report:2026-11-20");
    assert_eq!((enqueues[2].job.as_str(), enqueues[2].tx), ("reindex", 5));
    assert!(calls.iter().any(|c| matches!(c, syscall::Call::JobCancel(c) if c.run_id == "run-remind_unpaid" && c.request_id == 77)));
}

#[tokio::test]
async fn enqueue_checks_the_job_and_the_payload_first() {
    let (ctx, seen) = ctx_with_a_platform();
    let e = ctx.jobs().enqueue(NotAJob {}).await.unwrap_err();
    assert_eq!((e.status(), e.code()), (400, "invalid"));
    let e = ctx
        .jobs()
        .enqueue(Reindex {
            how: "x".repeat(70_000),
        })
        .await
        .unwrap_err();
    assert_eq!((e.status(), e.code()), (413, "payload_too_large"));
    assert!(seen.lock().unwrap().is_empty());
    // a test context takes the payload's name in snake_case; it has no platform
    let e = Ctx::test("orders", "1.0.0")
        .jobs()
        .enqueue(RemindUnpaid::default())
        .await
        .unwrap_err();
    assert_eq!(e.code(), "unavailable");
}

/// Runs `JobRun`s on a service and collects what it sends.
struct Pool {
    service: Service,
    link: Arc<Link>,
    tx: mpsc::Sender<pb::PluginMessage>,
    rx: mpsc::Receiver<pb::PluginMessage>,
    runs: crate::session::Runs,
    slots: Arc<Semaphore>,
}

impl Pool {
    fn new(p: Plugin) -> Self {
        let service = Service::new(p).unwrap();
        let (tx, rx) = mpsc::channel(64);
        Self {
            slots: Arc::new(Semaphore::new(service.job_slots() as usize)),
            link: Arc::new(Link::new(tx.clone())),
            service,
            tx,
            rx,
            runs: Arc::default(),
        }
    }

    fn start(&self, job: &str, run_id: &str, payload: &str) {
        self.service.start_run(
            pb::JobRun {
                run_id: run_id.into(),
                job: job.into(),
                attempt: 2,
                max_attempts: 4,
                payload: payload.into(),
                principal: Some(pb::Principal {
                    user_id: "u-7".into(),
                    kind: pb::principal::Kind::Person as i32,
                    ..Default::default()
                }),
                scheduled_at_unix_ms: 1_800_000_000_000,
                timeout_ms: 120_000,
                request_id: 900,
            },
            self.link.clone(),
            self.tx.clone(),
            &self.runs,
            &self.slots,
            false,
        );
    }

    /// The next `JobDone`, with the log lines sent before it.
    async fn done(&mut self) -> (pb::JobDone, Vec<pb::PluginLog>) {
        let mut lines = vec![];
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(5), self.rx.recv())
                .await
                .unwrap()
                .unwrap();
            match msg.kind {
                Some(plugin_message::Kind::JobDone(d)) => return (d, lines),
                Some(plugin_message::Kind::PluginLog(l)) => lines.push(l),
                other => panic!("unexpected {other:?}"),
            }
        }
    }
}

fn error_of(d: &pb::JobDone) -> (String, bool) {
    match d.outcome.as_ref().unwrap() {
        pb::job_done::Outcome::Error(e) => (e.code.clone(), e.permanent),
        pb::job_done::Outcome::Ok(_) => ("ok".into(), false),
    }
}

#[tokio::test]
async fn a_run_answers_its_outcome() {
    let _log = tracing::subscriber::set_default({
        use tracing_subscriber::layer::SubscriberExt as _;
        tracing_subscriber::registry().with(crate::runlog::run_log_layer())
    });
    let mut pool = Pool::new(plugin());

    pool.start("remind_unpaid", "run-0", r#"{"olderThanDays": 3}"#);
    let (d, lines) = pool.done().await;
    assert_eq!((d.run_id.as_str(), d.attempt), ("run-0", 2));
    assert_eq!(error_of(&d), ("ok".into(), false));
    // the handler's line, with the run's request id
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].request_id, 900);
    assert_eq!(lines[0].message, "reminding days=3");

    for (how, code, permanent) in [
        ("info", "ok", false),
        ("permanent", "permanent", true),
        ("invalid", "invalid", true),
        ("busy", "unavailable", false),
        ("panic", "internal", false),
    ] {
        pool.start("reindex", "run-1", &json!({ "how": how }).to_string());
        let (d, _) = pool.done().await;
        assert_eq!(error_of(&d), (code.into(), permanent), "{how}");
    }
    // a payload the job cannot read fails for good
    for payload in ["not json", r#"{"how": 5}"#] {
        pool.start("reindex", "run-2", payload);
        let (d, _) = pool.done().await;
        assert_eq!(error_of(&d), ("payload_invalid".into(), true), "{payload}");
    }
    // a job this version does not have is retried
    pool.start("gone", "run-3", "{}");
    let (d, _) = pool.done().await;
    assert_eq!(error_of(&d), ("unavailable".into(), false));
    assert!(pool.runs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_abort_drops_the_handler() {
    let mut pool = Pool::new(plugin().job_slots(1));
    pool.start("reindex", "run-1", r#"{"how": "hang"}"#);
    tokio::time::sleep(Duration::from_millis(20)).await;
    // its one slot is taken
    pool.start("reindex", "run-2", "{}");
    let (d, _) = pool.done().await;
    assert_eq!(
        (d.run_id.as_str(), error_of(&d)),
        ("run-2", ("unavailable".into(), false))
    );
    let stop = pool.runs.lock().unwrap().remove("run-1").unwrap();
    stop.send(pb::job_abort::Reason::Timeout as i32).unwrap();
    let (d, _) = pool.done().await;
    assert_eq!(
        (d.run_id.as_str(), error_of(&d)),
        ("run-1", ("timeout".into(), false))
    );
    // the slot is free again; an unknown reason is a cancel
    pool.start("reindex", "run-3", r#"{"how": "hang"}"#);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let stop = pool.runs.lock().unwrap().remove("run-3").unwrap();
    stop.send(99).unwrap();
    let (d, _) = pool.done().await;
    assert_eq!(error_of(&d), ("cancelled".into(), false));
}

#[tokio::test]
async fn a_handler_unit_test_sees_its_run() {
    let ctx = Ctx::test("orders", "1.0.0").with_run("run-9", 3, 3);
    assert_eq!(ctx.job().run_id(), "run-9");
    assert!(ctx.job().is_last_attempt());
    assert!(!Ctx::test("orders", "1.0.0").job().is_run());
}
