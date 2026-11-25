//! The conformance queries over views, as the three SDKs build them on the analytic engine:
//! `sdk/conformance/query/weekly.json`, the recent tasks of the last week whose month falls in 2026, counted by
//! month and stage; `sdk/conformance/query/estimates.json`, the long estimates of at least 20.50 hours, a decimal
//! column compared with a fractional decimal (sent as its text).

use celobus_sdk::prelude::*;
use celobus_sdk::query::count;
use serde_json::Value;

use crate::tree::tree_json;
use crate::{LongEstimate, RecentTask};

/// The conformance query over the view `recent_task`, from `ctx.analytics()`.
fn weekly(ctx: &Ctx) -> celobus_sdk::query::Selected<(Date, Option<String>, i64)> {
    ctx.analytics()
        .query::<RecentTask>()
        .filter(
            RecentTask::day.ge_col(days_ago(7))
                & RecentTask::day
                    .month()
                    .between("2026-01-01".parse().unwrap(), "2026-12-01".parse().unwrap()),
        )
        .group_by((RecentTask::day.month(), RecentTask::stage))
        .select((RecentTask::day.month(), RecentTask::stage, count()))
        .order_by(RecentTask::day.month().asc())
        .limit(12)
}

/// The conformance query over the view `long_estimate`, from `ctx.analytics()`.
fn estimates(ctx: &Ctx) -> celobus_sdk::query::Selected<(String, Decimal)> {
    ctx.analytics()
        .query::<LongEstimate>()
        .filter(LongEstimate::hours.ge("20.50".parse().unwrap()))
        .select((LongEstimate::title, LongEstimate::hours))
        .order_by(LongEstimate::hours.desc())
}

#[test]
fn the_conformance_view_query_is_the_shared_tree() {
    let ctx = Ctx::test("jobsdemo", "0.1.0");
    let expected: Value = serde_json::from_str(include_str!("../../../query/weekly.json")).unwrap();
    assert_eq!(tree_json(weekly(&ctx).tree()), expected);
}

#[test]
fn the_conformance_decimal_query_is_the_shared_tree() {
    let ctx = Ctx::test("jobsdemo", "0.1.0");
    let expected: Value =
        serde_json::from_str(include_str!("../../../query/estimates.json")).unwrap();
    assert_eq!(tree_json(estimates(&ctx).tree()), expected);
}

#[test]
fn the_conformance_lake_query_is_the_shared_tree() {
    let ctx = Ctx::test("jobsdemo", "0.1.0");
    let got = tree_json(crate::lake::task_days(&ctx).tree());
    if std::env::var_os("CONFORMANCE_PRINT").is_some() {
        println!("{}", serde_json::to_string_pretty(&got).unwrap());
    }
    let expected: Value = serde_json::from_str(include_str!("../../../query/lake.json")).unwrap();
    assert_eq!(got, expected);
}
