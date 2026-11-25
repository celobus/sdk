//! Models of the lake in the SDK: what a plugin declares (`Model`, `Plugin::model`, `.parse`) and what a refresh sends,
//! against a fake platform.

use celobus_proto as pb;
use pb::{syscall, syscall_result};
use serde_json::{Value, json};

use crate::db_tests::ctx_with;
use crate::lake::Refreshed;
use crate::prelude::*;
use crate::query::{count_distinct, sum};

const ANALYST: Role = Role::new("analyst").title("Analyst");
const RAW: Layer = Layer::new("raw").title("As it arrived");
const CORE: Layer = Layer::new("core").title("Cleaned");
const MART: Layer = Layer::new("mart").title("For reports");

#[derive(LakeTable, Clone, Debug, PartialEq)]
#[lake(layer = RAW, name = "stock_file", partition(loaded_on))]
pub struct StockFileRow {
    pub loaded_on: Date,
    pub sku: String,
    pub on_hand: String,
}

#[derive(LakeTable, Clone, Debug, PartialEq)]
#[lake(layer = CORE, partition(day))]
pub struct Stock {
    pub day: Date,
    pub sku: String,
    pub on_hand: Decimal,
}

#[derive(LakeTable, Clone, Debug, PartialEq)]
#[lake(layer = MART, name = "stock_by_month", partition(month))]
pub struct StockByMonth {
    pub month: Date,
    pub skus: i64,
    pub on_hand: Decimal,
}

fn stock() -> Model<Stock> {
    Model::<Stock>::from::<StockFileRow>()
        .filter(!StockFileRow::sku.eq(String::new()))
        .col(Stock::on_hand, StockFileRow::on_hand.parse::<Decimal>())
        .col(Stock::day, StockFileRow::loaded_on)
        .col(Stock::sku, StockFileRow::sku)
}

fn by_month() -> Model<StockByMonth> {
    Model::<StockByMonth>::from::<Stock>()
        .group_by(Stock::day.month())
        .col(StockByMonth::month, Stock::day.month())
        .col(StockByMonth::skus, count_distinct(Stock::sku))
        .col(StockByMonth::on_hand, sum(Stock::on_hand))
}

fn plugin() -> Plugin {
    Plugin::new("retail", "1.0.0")
        .role(ANALYST)
        .layer(RAW)
        .layer(CORE)
        .layer(MART)
        .lake_table::<StockFileRow>()
        .lake_read_as_system::<StockFileRow>()
        .lake_table::<Stock>()
        .lake_read_as_system::<Stock>()
        .model(stock())
        .lake_table::<StockByMonth>()
        .model(by_month())
        .lake_read::<StockByMonth>([ANALYST])
        .lake_export_to_models::<StockByMonth>()
}

#[test]
fn a_plugin_declares_its_models() {
    let json: Value = serde_json::from_str(&plugin().describe_json().unwrap()).unwrap();
    let col = |s: &str, f: &str| json!({ "column": { "source": s, "field": f } });
    assert_eq!(
        json["lake_models"],
        json!([
            { "table": "stock",
              "sources": [{ "entity": "stock_file", "alias": "stock_file" }],
              "filter": { "not": { "compare": { "left": col("stock_file", "sku"), "op": "eq", "values": [""] } } },
              // in the order of the table's fields, whatever the order of `col`
              "columns": [
                { "name": "day", "value": col("stock_file", "loaded_on") },
                { "name": "sku", "value": col("stock_file", "sku") },
                { "name": "on_hand",
                  "value": { "column": { "source": "stock_file", "field": "on_hand", "parse": "decimal" } } }
              ] },
            { "table": "stock_by_month",
              "sources": [{ "entity": "stock", "alias": "stock" }],
              "group_by": [{ "source": "stock", "field": "day", "bucket": "month" }],
              "columns": [
                { "name": "month", "value": { "column": { "source": "stock", "field": "day", "bucket": "month" } } },
                { "name": "skus",
                  "value": { "aggregate": { "fn": "count_distinct", "column": { "source": "stock", "field": "sku" } } } },
                { "name": "on_hand",
                  "value": { "aggregate": { "fn": "sum", "column": { "source": "stock", "field": "on_hand" } } } }
              ] }
        ])
    );
    // the table other plugins' models may copy
    let t = json["lake_tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "stock_by_month")
        .unwrap();
    assert_eq!(t["export_models"], true);
    assert!(t.get("export").is_none(), "apart from export: {t}");
    // the platform's check reads the same
    let checked = celobus_descriptor::check(json.to_string().as_bytes()).unwrap();
    assert_eq!(checked.descriptor.lake_models.len(), 2);
    let plain: Value =
        serde_json::from_str(&Plugin::new("plain", "1.0.0").describe_json().unwrap()).unwrap();
    assert!(plain.get("lake_models").is_none());
}

#[test]
fn mistakes_of_a_model_are_reported() {
    let codes = |p: Plugin| -> Vec<String> {
        p.descriptor()
            .unwrap_err()
            .0
            .into_iter()
            .map(|e| e.code.to_string())
            .collect()
    };
    // a column given twice
    let twice = Model::<Stock>::from::<StockFileRow>()
        .col(Stock::day, StockFileRow::loaded_on)
        .col(Stock::day, StockFileRow::loaded_on);
    let c = codes(
        Plugin::new("retail", "1.0.0")
            .layer(RAW)
            .layer(CORE)
            .lake_table::<StockFileRow>()
            .lake_table::<Stock>()
            .model(twice),
    );
    assert!(c.contains(&"E_LAKE_MODEL_DEFINE".to_string()), "{c:?}");
    // a field left without a column, and a model of a table the plugin does not declare
    let short = Model::<Stock>::from::<StockFileRow>().col(Stock::day, StockFileRow::loaded_on);
    let c = codes(
        Plugin::new("retail", "1.0.0")
            .layer(RAW)
            .lake_table::<StockFileRow>()
            .model(short),
    );
    assert!(c.contains(&"E_LAKE_MODEL_TABLE".to_string()), "{c:?}");
    // a model that reads its own table
    let own = Model::<Stock>::from::<Stock>()
        .col(Stock::day, Stock::day)
        .col(Stock::sku, Stock::sku)
        .col(Stock::on_hand, Stock::on_hand);
    let c = codes(
        Plugin::new("retail", "1.0.0")
            .layer(CORE)
            .lake_table::<Stock>()
            .model(own),
    );
    assert!(c.contains(&"E_LAKE_MODEL_CYCLE".to_string()), "{c:?}");
}

#[tokio::test]
async fn a_refresh_sends_its_table_and_partitions_and_reads_what_it_did() {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let saw = seen.clone();
    let (ctx, _) = ctx_with(move |call| match call {
        syscall::Call::LakeRefresh(r) => {
            saw.lock().unwrap().push(r.clone());
            Ok(syscall_result::Result::LakeRefreshed(pb::LakeRefreshed {
                rows: 12,
                as_of: if r.table == "stock" {
                    0
                } else {
                    1_767_225_600_000
                },
                stale_partitions: if r.filter.is_some() { 3 } else { 0 },
            }))
        }
        _ => Err(("unavailable: not here".into(), None)),
    });
    let out = ctx.lake().refresh::<Stock>().await.unwrap();
    assert_eq!(
        out,
        Refreshed {
            rows: 12,
            as_of: None,
            stale_partitions: 0
        }
    );
    let day: Date = "2026-01-01".parse().unwrap();
    let out = ctx
        .lake()
        .refresh::<StockByMonth>()
        .partition(StockByMonth::month.eq(day))
        .await
        .unwrap();
    assert_eq!(out.as_of.unwrap().to_rfc3339(), "2026-01-01T00:00:00+00:00");
    assert_eq!(out.stale_partitions, 3);
    let calls = seen.lock().unwrap().clone();
    assert_eq!(calls[0].table, "stock");
    assert!(calls[0].filter.is_none(), "whole");
    assert_eq!(calls[1].table, "stock_by_month");
    assert_eq!(calls[1].request_id, 77);
    let f = calls[1].filter.as_ref().unwrap();
    let Some(pb::db_filter::Node::Condition(c)) = &f.node else {
        panic!("{f:?}")
    };
    assert_eq!(c.field, "month");
}

#[tokio::test]
async fn a_refresh_s_refusals_carry_their_status() {
    for (answer, status) in [
        (
            "lake_model_rows_limit: the model computes more than 1000000 rows",
            422,
        ),
        (
            "lake_model_parse: f.on_hand holds a text that is not a decimal",
            422,
        ),
        ("lake_model_changed: the model of stock changed", 409),
        ("capability_missing: the model reads product", 403),
    ] {
        let (ctx, _) = ctx_with(move |_| Err((answer.to_string(), None)));
        let err = ctx.lake().refresh::<Stock>().await.unwrap_err();
        assert_eq!(
            (err.code(), err.status()),
            (answer.split(':').next().unwrap(), status),
            "{err}"
        );
    }
}
