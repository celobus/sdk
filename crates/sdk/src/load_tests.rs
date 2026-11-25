//! Entities stored in OLAP in the SDK: what a plugin declares (`#[entity(storage = olap, key(..))]`, `Load`,
//! `Plugin::load`) and what a request for a snapshot and its ticket send and read, against a fake platform.

use std::sync::atomic::{AtomicU32, Ordering};

use celobus_proto as pb;
use pb::{syscall, syscall_result};
use serde_json::{Value, json};

use crate::TicketState;
use crate::db_tests::ctx_with;
use crate::prelude::*;

const ANALYST: Role = Role::new("analyst").title("Analyst");
const MART: Layer = Layer::new("mart").title("For reports");

#[derive(LakeTable, Clone, Debug, PartialEq)]
#[lake(layer = MART, name = "metrics", partition(day))]
pub struct MetricRow {
    pub day: Date,
    pub store_id: Id<Store>,
    pub metric: String,
    pub value: String,
}

/// A store.
#[derive(Entity, Clone, Debug, PartialEq)]
pub struct Store {
    pub id: Id<Store>,
    pub name: String,
}

/// One metric of one store per day.
#[derive(Entity, Clone, Debug, PartialEq)]
#[entity(storage = olap, key(day, store, metric))]
pub struct StoreMetric {
    pub id: Id<StoreMetric>,
    pub day: Date,
    pub store: Id<Store>,
    pub metric: String,
    pub value: Option<Decimal>,
}

fn store_metric_load() -> Load<StoreMetric> {
    Load::<StoreMetric>::from::<MetricRow>()
        .filter(MetricRow::day.ge_col(crate::query::days_ago(400)))
        .col(StoreMetric::value, MetricRow::value.parse::<Decimal>())
        .col(StoreMetric::day, MetricRow::day)
        .col(StoreMetric::store, MetricRow::store_id)
        .col(StoreMetric::metric, MetricRow::metric)
}

fn plugin() -> Plugin {
    Plugin::new("retail", "1.0.0")
        .role(ANALYST)
        .layer(MART)
        .lake_table::<MetricRow>()
        .lake_read_as_system::<MetricRow>()
        .entity::<Store>()
        .access::<Store>(Access::build(|a| a.read(ANALYST)))
        .entity::<StoreMetric>()
        .access::<StoreMetric>(Access::build(|a| {
            let store = a.relation("store", Rel::column(StoreMetric::store));
            a.read(store.via(Perm::READ) | ANALYST);
            a.mask(StoreMetric::value, [ANALYST]);
        }))
        .load(store_metric_load())
}

#[test]
fn a_plugin_declares_an_entity_stored_in_olap_and_its_load() {
    let json: Value = serde_json::from_str(&plugin().describe_json().unwrap()).unwrap();
    let e = json["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "store_metric")
        .unwrap();
    assert_eq!(e["storage"], "olap");
    assert_eq!(e["olap_key"], json!(["day", "store", "metric"]));
    let store = json["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "store")
        .unwrap();
    assert!(store.get("storage").is_none() && store.get("olap_key").is_none());
    let col = |s: &str, f: &str| json!({ "column": { "source": s, "field": f } });
    assert_eq!(
        json["loads"],
        json!([{
            "entity": "store_metric",
            "sources": [{ "entity": "metrics", "alias": "metrics" }],
            "filter": { "compare": { "left": col("metrics", "day"), "op": "ge", "right": { "days_ago": 400 } } },
            "columns": [
                { "name": "day", "value": col("metrics", "day") },
                { "name": "store", "value": col("metrics", "store_id") },
                { "name": "metric", "value": col("metrics", "metric") },
                { "name": "value", "value": { "column": { "source": "metrics", "field": "value", "parse": "decimal" } } }
            ]
        }])
    );
    // the descriptor holds: what `celobus plugin check` checks
    let d = celobus_descriptor::parse(plugin().describe_json().unwrap().as_bytes()).unwrap();
    assert_eq!(celobus_descriptor::validate(&d), vec![]);
}

#[test]
fn a_load_that_does_not_hold_is_reported_at_build() {
    // a required field left out
    let bad = Load::<StoreMetric>::from::<MetricRow>()
        .col(StoreMetric::day, MetricRow::day)
        .col(StoreMetric::store, MetricRow::store_id);
    let e = Plugin::new("retail", "1.0.0")
        .layer(MART)
        .lake_table::<MetricRow>()
        .lake_read_as_system::<MetricRow>()
        .entity::<Store>()
        .entity::<StoreMetric>()
        .load(bad)
        .describe_json()
        .unwrap_err()
        .to_string();
    assert!(e.contains("E_LOAD_COLUMN"), "{e}");
    // an entity stored in OLAP without its load
    let e = Plugin::new("retail", "1.0.0")
        .entity::<Store>()
        .entity::<StoreMetric>()
        .describe_json()
        .unwrap_err()
        .to_string();
    assert!(e.contains("E_OLAP_LOAD"), "{e}");
}

#[tokio::test]
async fn a_request_answers_a_ticket_and_wait_follows_it_to_its_end() {
    let asked = std::sync::Arc::new(AtomicU32::new(0));
    let n = asked.clone();
    let (ctx, seen) = ctx_with(move |call| match call {
        syscall::Call::AnalyticsLoad(r) => {
            assert_eq!(r.entities, vec!["store_metric".to_string()]);
            Ok(syscall_result::Result::AnalyticsTicket(
                pb::AnalyticsTicket {
                    ticket: "t1".into(),
                    state: pb::analytics_ticket::State::Pending as i32,
                    entities: r.entities.clone(),
                    ..Default::default()
                },
            ))
        }
        syscall::Call::AnalyticsLoadStatus(s) => {
            // a request's wait asks for what is left before its deadline, less a margin
            assert_eq!(s.ticket.as_str(), "t1");
            assert!(s.wait_ms > 0 && s.wait_ms <= 3_000, "{}", s.wait_ms);
            let done = n.fetch_add(1, Ordering::Relaxed) >= 1;
            Ok(syscall_result::Result::AnalyticsTicket(
                pb::AnalyticsTicket {
                    ticket: "t1".into(),
                    state: if done {
                        pb::analytics_ticket::State::Done
                    } else {
                        pb::analytics_ticket::State::Building
                    } as i32,
                    generation: 7,
                    loads: vec![pb::AnalyticsLoadOutcome {
                        entity: "store_metric".into(),
                        rows: 3,
                        loaded_at: 1_795_000_000_000,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ))
        }
        other => panic!("{other:?}"),
    });
    let t = ctx
        .analytics()
        .snapshot()
        .load::<StoreMetric>()
        .load::<StoreMetric>()
        .request()
        .await
        .unwrap();
    assert_eq!((t.id(), t.last().state), ("t1", TicketState::Pending));
    let done = t.wait().await.unwrap();
    assert_eq!((done.state, done.generation), (TicketState::Done, 7));
    assert_eq!(done.loads[0].rows, 3);
    assert!(done.loads[0].loaded_at.is_some());
    assert_eq!(seen.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn a_failed_ticket_is_its_load_s_error_and_refusals_carry_their_status() {
    let (ctx, _) = ctx_with(move |call| match call {
        syscall::Call::AnalyticsLoad(_) => Ok(syscall_result::Result::AnalyticsTicket(
            pb::AnalyticsTicket {
                ticket: "t2".into(),
                ..Default::default()
            },
        )),
        _ => Ok(syscall_result::Result::AnalyticsTicket(
            pb::AnalyticsTicket {
                ticket: "t2".into(),
                state: pb::analytics_ticket::State::Failed as i32,
                code: "analytic_load_duplicate".into(),
                error: "the load gives 2 keys more than once".into(),
                ..Default::default()
            },
        )),
    });
    let t = ctx
        .analytics()
        .snapshot()
        .load::<StoreMetric>()
        .request()
        .await
        .unwrap();
    let e = t.wait().await.unwrap_err();
    assert_eq!((e.status(), e.code()), (422, "analytic_load_duplicate"));
    // a failure without a code is the snapshot's, with its status
    let blank = crate::TicketStatus {
        code: String::new(),
        ..t_failed()
    };
    let e = blank.error().unwrap();
    assert_eq!((e.status(), e.code()), (503, "analytic_load_failed"));
    // an ordinary entity is no load: refused before anything is sent
    let e = ctx
        .analytics()
        .snapshot()
        .load::<Store>()
        .request()
        .await
        .err()
        .unwrap();
    assert_eq!((e.status(), e.code()), (400, "invalid"));
    for (answer, status, code) in [
        ("analytic_load_limit: the bound", 429, "analytic_load_limit"),
        ("not_loaded: no load yet", 409, "not_loaded"),
    ] {
        let (ctx, _) = ctx_with(move |_| Err((answer.to_string(), None)));
        let e = ctx
            .analytics()
            .snapshot()
            .load::<StoreMetric>()
            .request()
            .await
            .err()
            .unwrap();
        assert_eq!((e.status(), e.code()), (status, code), "{answer}");
    }
}

fn t_failed() -> crate::TicketStatus {
    crate::TicketStatus {
        id: "t".into(),
        state: TicketState::Failed,
        generation: 0,
        entities: vec![],
        loads: vec![],
        code: "analytic_load_duplicate".into(),
        error: "twice".into(),
        requested_at: None,
        finished_at: None,
        not_before: None,
    }
}

#[tokio::test]
async fn a_wait_in_a_request_ends_before_its_deadline_with_the_ticket_pending() {
    let (ctx, _) = ctx_with(move |call| match call {
        syscall::Call::AnalyticsLoad(r) => Ok(syscall_result::Result::AnalyticsTicket(
            pb::AnalyticsTicket {
                ticket: "t3".into(),
                entities: r.entities.clone(),
                not_before: 1_795_000_600_000,
                ..Default::default()
            },
        )),
        syscall::Call::AnalyticsLoadStatus(s) => {
            // the platform waits what it was asked, the ticket still pending
            std::thread::sleep(std::time::Duration::from_millis(s.wait_ms.min(1_000) as u64));
            Ok(syscall_result::Result::AnalyticsTicket(
                pb::AnalyticsTicket {
                    ticket: "t3".into(),
                    not_before: 1_795_000_600_000,
                    ..Default::default()
                },
            ))
        }
        other => panic!("{other:?}"),
    });
    let t = ctx
        .analytics()
        .snapshot()
        .load::<StoreMetric>()
        .request()
        .await
        .unwrap();
    assert_eq!(
        t.last().not_before.map(|t| t.timestamp_millis()),
        Some(1_795_000_600_000)
    );
    let began = std::time::Instant::now();
    let e = t.wait().await.unwrap_err();
    // before the request's 5 s deadline, with time to answer
    assert!(
        began.elapsed() < std::time::Duration::from_secs(4),
        "{:?}",
        began.elapsed()
    );
    assert_eq!((e.status(), e.code()), (503, "analytic_load_pending"));
    let v = serde_json::to_value(e.details()).unwrap();
    assert_eq!(v["ticket"], "t3");
    assert_eq!(v["state"], "pending");
    assert_eq!(v["not_before"], 1_795_000_600_000i64);
}
