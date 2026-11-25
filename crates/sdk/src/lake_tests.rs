//! The Data Lake in the SDK: what a plugin declares of its layers and lake tables, and what a write and a query send,
//! against a fake platform.

use std::sync::{Arc, Mutex};

use celobus_proto as pb;
use pb::{lake_write::Mode, syscall, syscall_result};
use serde_json::{Value, json};

use crate::db_tests::ctx_with;
use crate::lake::Written;
use crate::prelude::*;

const ANALYST: Role = Role::new("analyst").title("Analyst");
const RAW: Layer = Layer::new("raw").title("As it arrived");
const CORE: Layer = Layer::new("core").title("Cleaned");

/// Stock as the ERP sends it.
#[derive(LakeTable, Clone, Debug, PartialEq)]
#[lake(layer = RAW, name = "stock_file", partition(loaded_on))]
pub struct StockFileRow {
    pub loaded_on: Date,
    pub sku: String,
    pub on_hand: String,
}

#[derive(LakeTable, Clone, Debug, PartialEq)]
#[lake(layer = CORE, partition(day), key(day, sku))]
pub struct Stock {
    pub day: Date,
    pub sku: String,
    pub on_hand: Decimal,
    pub reserved: Option<i64>,
}

/// Another plugin's exported table.
#[derive(LakeTable, Clone, Debug, PartialEq)]
#[lake(plugin = "erp", name = "deliveries")]
pub struct Delivery {
    pub day: Date,
    pub sku: String,
}

fn plugin() -> Plugin {
    Plugin::new("retail", "1.0.0")
        .role(ANALYST)
        .layer(RAW)
        .layer(CORE)
        .lake_table::<StockFileRow>()
        .lake_table::<Stock>()
        .lake_read::<Stock>([ANALYST])
        .lake_read_as_system::<Stock>()
        .lake_export::<Stock>()
        .lake_dependency::<Delivery>()
}

#[test]
fn a_plugin_declares_its_layers_and_lake_tables() {
    let json: Value = serde_json::from_str(&plugin().describe_json().unwrap()).unwrap();
    assert_eq!(
        json["layers"],
        json!([{ "name": "raw", "title": "As it arrived" }, { "name": "core", "title": "Cleaned" }])
    );
    assert_eq!(
        json["lake_tables"],
        json!([
            { "name": "stock_file", "layer": "raw", "message": "StockFileRow", "description": "Stock as the ERP sends it.",
              "partition": ["loaded_on"] },
            { "name": "stock", "layer": "core", "message": "Stock", "partition": ["day"], "key": ["day", "sku"],
              "read": ["analyst"], "system_read": true, "export": true }
        ])
    );
    assert_eq!(
        json["dependencies"],
        json!([{ "plugin": "erp", "calls": [], "lake": ["deliveries"] }])
    );
    // the platform's check reads the same
    let checked = celobus_descriptor::check(json.to_string().as_bytes()).unwrap();
    assert_eq!(checked.descriptor.lake_tables.len(), 2);
    // nothing of the lake for a plugin without it
    let plain: Value =
        serde_json::from_str(&Plugin::new("plain", "1.0.0").describe_json().unwrap()).unwrap();
    assert!(plain.get("layers").is_none() && plain.get("lake_tables").is_none());
}

#[test]
fn mistakes_of_the_lake_declarations_are_reported() {
    let codes = |p: Plugin| -> Vec<String> {
        p.descriptor()
            .unwrap_err()
            .0
            .into_iter()
            .map(|e| e.code.to_string())
            .collect()
    };
    // a layer not declared, a read of a table not declared, another plugin's table as one's own
    let c = codes(
        Plugin::new("retail", "1.0.0")
            .role(ANALYST)
            .layer(RAW)
            .lake_table::<Stock>(),
    );
    assert!(c.contains(&"E_LAKE_TABLE_LAYER".to_string()), "{c:?}");
    let c = codes(
        Plugin::new("retail", "1.0.0")
            .role(ANALYST)
            .layer(CORE)
            .lake_read::<Stock>([ANALYST]),
    );
    assert!(c.contains(&"E_LAKE_DEFINE".to_string()), "{c:?}");
    let c = codes(Plugin::new("retail", "1.0.0").lake_table::<Delivery>());
    assert!(c.contains(&"E_LAKE_DEFINE".to_string()), "{c:?}");
    // a read role the plugin does not declare
    let c = codes(
        Plugin::new("retail", "1.0.0")
            .layer(CORE)
            .lake_table::<Stock>()
            .lake_read::<Stock>([ANALYST]),
    );
    assert!(c.contains(&"E_LAKE_ROLE".to_string()), "{c:?}");
}

fn day(s: &str) -> Date {
    s.parse().unwrap()
}

/// What the fake platform saw: the chunks of the writes.
type Seen = Arc<Mutex<Vec<syscall::Call>>>;

fn lake_platform() -> (Ctx, Seen) {
    let next = Arc::new(Mutex::new(0u64));
    ctx_with(move |call| match call {
        syscall::Call::LakeWrite(w) => {
            let mut n = next.lock().unwrap();
            *n += w.rows.len() as u64;
            Ok(syscall_result::Result::LakeWritten(pb::LakeWritten {
                write: 41,
                rows: *n,
                committed: w.last && w.key != "seen",
                skipped: w.last && w.key == "seen",
            }))
        }
        syscall::Call::Query(_) => Ok(syscall_result::Result::DbRows(pb::DbRows {
            rows: vec![bytes::Bytes::from(
                r#"{"day":"2026-01-01","sku":"a","onHand":"1.5"}"#,
            )],
            ..Default::default()
        })),
        _ => Err(("unavailable: not here".into(), None)),
    })
}

#[tokio::test]
async fn a_write_goes_in_chunks_and_commits_with_the_last() {
    let (ctx, seen) = lake_platform();
    let rows: Vec<StockFileRow> = (0..25_001)
        .map(|i| StockFileRow {
            loaded_on: day("2026-01-01"),
            sku: format!("s{i}"),
            on_hand: "1".into(),
        })
        .collect();
    let out = ctx
        .lake()
        .table::<StockFileRow>()
        .replace(StockFileRow::loaded_on.eq(day("2026-01-01")), rows)
        .await
        .unwrap();
    assert_eq!(
        out,
        Written {
            rows: 25_001,
            skipped: false
        }
    );
    let calls = seen.lock().unwrap().clone();
    let writes: Vec<&pb::LakeWrite> = calls
        .iter()
        .map(|c| match c {
            syscall::Call::LakeWrite(w) => w,
            _ => panic!("{c:?}"),
        })
        .collect();
    assert_eq!(
        writes
            .iter()
            .map(|w| (w.rows.len(), w.write, w.last))
            .collect::<Vec<_>>(),
        [(10_000, 0, false), (10_000, 41, false), (5_001, 41, true)]
    );
    let first = writes[0];
    assert_eq!(
        (first.table.as_str(), first.mode),
        ("stock_file", Mode::Replace as i32)
    );
    assert!(first.filter.is_some() && writes[1].filter.is_none());
    assert_eq!(first.request_id, 77);
    let row: Value = serde_json::from_slice(&first.rows[0]).unwrap();
    assert_eq!(
        row,
        json!({ "loadedOn": "2026-01-01", "sku": "s0", "onHand": "1" })
    );
}

#[tokio::test]
async fn an_append_carries_its_key_and_a_run_s_default_one() {
    let (ctx, seen) = lake_platform();
    let row = Stock {
        day: day("2026-01-01"),
        sku: "a".into(),
        on_hand: "1.5".parse().unwrap(),
        reserved: None,
    };
    let out = ctx
        .lake()
        .table::<Stock>()
        .append([row.clone()])
        .key("seen")
        .await
        .unwrap();
    assert!(out.skipped);
    // outside a run without a key: refused before anything is sent
    let err = ctx
        .lake()
        .table::<Stock>()
        .append([row.clone()])
        .await
        .unwrap_err();
    assert_eq!(err.code(), "invalid");
    // in a run: its id and the append's number, the same in every attempt
    for attempt in 1..=2 {
        let run = ctx.with_run("run-7", attempt, 3);
        run.lake()
            .table::<Stock>()
            .append([row.clone()])
            .await
            .unwrap();
        run.lake()
            .table::<Stock>()
            .append([row.clone()])
            .await
            .unwrap();
    }
    ctx.lake().table::<Stock>().merge([row]).await.unwrap();
    let keys: Vec<(i32, String)> = seen
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            syscall::Call::LakeWrite(w) => Some((w.mode, w.key.clone())),
            _ => None,
        })
        .collect();
    let a = Mode::Append as i32;
    assert_eq!(
        keys,
        [
            (a, "seen".into()),
            (a, "run-7#1".into()),
            (a, "run-7#2".into()),
            (a, "run-7#1".into()),
            (a, "run-7#2".into()),
            (Mode::Merge as i32, String::new())
        ]
    );
}

#[tokio::test]
async fn a_lake_query_reads_lake_tables_by_name() {
    let (ctx, seen) = lake_platform();
    let rows = ctx
        .lake()
        .query::<Stock>()
        .filter(Stock::sku.eq("a".to_string()))
        .all()
        .await
        .unwrap();
    assert_eq!(rows[0].on_hand.to_string(), "1.5");
    let theirs = ctx.lake().query::<Delivery>().count().await;
    assert!(theirs.is_err(), "the fake answers a count with rows");
    let calls = seen.lock().unwrap().clone();
    let reads: Vec<&pb::QueryRead> = calls
        .iter()
        .filter_map(|c| match c {
            syscall::Call::Query(q) => Some(q),
            _ => None,
        })
        .collect();
    assert_eq!(reads[0].engine, pb::Engine::Lake as i32);
    assert_eq!(reads[0].query.as_ref().unwrap().sources[0].entity, "stock");
    assert_eq!(
        reads[1].query.as_ref().unwrap().sources[0].entity,
        "erp.deliveries"
    );
}

#[tokio::test]
async fn another_plugin_s_table_is_not_written() {
    let (ctx, _) = lake_platform();
    let err = ctx
        .lake()
        .table::<Delivery>()
        .merge([Delivery {
            day: day("2026-01-01"),
            sku: "a".into(),
        }])
        .await
        .unwrap_err();
    assert!(err.to_string().contains("another plugin"), "{err}");
}

#[test]
fn rows_past_a_million_are_refused_before_anything_is_sent() {
    let rows = (0..=crate::lake::WRITE_ROWS).map(|_| StockFileRow {
        loaded_on: day("2026-01-01"),
        sku: String::new(),
        on_hand: String::new(),
    });
    let err = crate::lake::chunks(rows).unwrap_err();
    assert_eq!(err.code(), "lake_rows_limit");
    // chunks by size too
    let big = "x".repeat(1 << 20);
    let rows = (0..9).map(|_| StockFileRow {
        loaded_on: day("2026-01-01"),
        sku: big.clone(),
        on_hand: String::new(),
    });
    let chunks = crate::lake::chunks(rows).unwrap();
    assert_eq!(chunks.iter().map(Vec::len).collect::<Vec<_>>(), [3, 3, 3]);
}

#[tokio::test]
async fn the_lake_s_refusals_carry_their_status() {
    for (answer, status) in [
        (
            "lake_busy: other writes of this table ran for more than 60 s; retry",
            503,
        ),
        ("lake_rows_limit: a write holds at most 1000000 rows", 422),
        ("lake_time_limit: the query took longer than 10 s", 422),
        ("lake_memory_limit: the query needs more memory", 422),
    ] {
        let (ctx, _) = ctx_with(move |_| Err((answer.to_string(), None)));
        let err = ctx
            .lake()
            .table::<Stock>()
            .merge([Stock {
                day: day("2026-01-01"),
                sku: "a".into(),
                on_hand: "1".parse().unwrap(),
                reserved: None,
            }])
            .await
            .unwrap_err();
        assert_eq!(
            (err.code(), err.status()),
            (answer.split(':').next().unwrap(), status),
            "{err}"
        );
    }
}
