//! Entities' histories in the SDK: what a plugin declares, what a history read and a model over a history send,
//! against a fake platform.

use std::sync::{Arc, Mutex};

use celobus_proto as pb;
use pb::{syscall, syscall_result};
use serde_json::{Value, json};

use crate::db_tests::ctx_with;
use crate::lake::history::{self, HistoryRow};
use crate::prelude::*;

const ANALYST: Role = Role::new("analyst").title("Analyst");
const AGENT: Role = Role::new("agent").title("Agent");
const AUDIT: Layer = Layer::new("audit").title("History of records");
const MART: Layer = Layer::new("mart").title("For reports");

#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TicketStatus {
    Open,
    Closed,
}

#[derive(Entity, Clone, Debug)]
#[entity(soft_delete, history(layer = AUDIT))]
pub struct Ticket {
    pub id: Id<Ticket>,
    pub title: String,
    #[col(history)]
    pub status: TicketStatus,
    #[col(history)]
    pub assignee: Option<Id<crate::platform::User>>,
    pub body: String,
}

/// Every column kept, but the encrypted one and the masked one.
#[derive(Entity, Clone, Debug)]
#[entity(history(layer = AUDIT, all))]
pub struct Contract {
    pub id: Id<Contract>,
    pub title: String,
    pub amount: Option<Decimal>,
    #[col(encrypted)]
    pub secret: Option<String>,
    pub notes: Option<String>,
}

/// Open tickets at the end of each day.
#[derive(LakeTable, Clone, Debug)]
#[lake(layer = MART, name = "open_by_day", partition(day))]
pub struct OpenByDay {
    pub day: Date,
    pub assignee: Option<Id<crate::platform::User>>,
    pub open: i64,
}

fn open_by_day() -> Model<OpenByDay> {
    Model::<OpenByDay>::from_history(history::history::<Ticket>().each_day())
        .filter(!Ticket::status.eq(TicketStatus::Closed))
        .group_by((history::day::<Ticket>(), Ticket::assignee))
        .col(OpenByDay::day, history::day::<Ticket>())
        .col(OpenByDay::assignee, Ticket::assignee)
        .col(OpenByDay::open, crate::query::count())
}

fn plugin() -> Plugin {
    Plugin::new("helpdesk", "1.0.0")
        .role(ANALYST)
        .role(AGENT)
        .layer(AUDIT)
        .layer(MART)
        .entity::<Ticket>()
        .history_read::<Ticket>([ANALYST])
        .history_read_as_system::<Ticket>()
        .history_export::<Ticket>()
        .entity::<Contract>()
        .access::<Contract>(Access::build(|a| {
            a.read(AGENT);
            a.mask(Contract::notes, [AGENT]);
        }))
        .lake_table::<OpenByDay>()
        .model(open_by_day())
}

#[test]
fn an_entity_declares_its_history_and_its_kept_columns() {
    let json: Value = serde_json::from_str(&plugin().describe_json().unwrap()).unwrap();
    let ticket = &json["entities"][0];
    assert_eq!(
        ticket["history"],
        json!({ "layer": "audit", "read": ["analyst"], "system_read": true, "export": true })
    );
    let kept: Vec<&str> = ticket["fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["history"] == json!(true))
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(kept, ["status", "assignee"]);
    // `all`: a flag on every column it covers, none on the encrypted and the masked one
    let contract = &json["entities"][1];
    assert_eq!(
        contract["history"],
        json!({ "layer": "audit", "all": true })
    );
    let kept: Vec<&str> = contract["fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["history"] == json!(true))
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(kept, ["title", "amount"]);
    // the message of the row carries no history flag
    let msg = json["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "Ticket")
        .unwrap();
    assert!(
        msg["fields"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f.get("history").is_none())
    );
    // the model over the history per day
    assert_eq!(
        json["lake_models"][0]["sources"],
        json!([{ "entity": "ticket", "alias": "ticket", "history": true, "each_day": true }])
    );
    let checked = celobus_descriptor::check(json.to_string().as_bytes()).unwrap();
    assert!(checked.descriptor.entities[0].history.is_some());
    // nothing of history for an entity without one
    let plain: Value = serde_json::from_str(
        &Plugin::new("plain", "1.0.0")
            .layer(AUDIT)
            .describe_json()
            .unwrap(),
    )
    .unwrap();
    assert!(plain["entities"].as_array().unwrap().is_empty());
}

#[test]
fn mistakes_of_a_history_are_reported() {
    let codes = |p: Plugin| -> Vec<String> {
        p.descriptor()
            .unwrap_err()
            .0
            .into_iter()
            .map(|e| e.code.to_string())
            .collect()
    };
    // its layer is not declared
    let c = codes(Plugin::new("helpdesk", "1.0.0").entity::<Ticket>());
    assert!(c.contains(&"E_HISTORY_LAYER".to_string()), "{c:?}");
    // a read of a history an entity does not keep
    #[derive(Entity, Clone, Debug)]
    pub struct Plain {
        pub id: Id<Plain>,
        pub name: String,
    }
    let c = codes(
        Plugin::new("helpdesk", "1.0.0")
            .role(ANALYST)
            .entity::<Plain>()
            .history_read::<Plain>([ANALYST]),
    );
    assert!(c.contains(&"E_HISTORY_DECLARE".to_string()), "{c:?}");
    // a role the plugin does not declare
    let c = codes(
        Plugin::new("helpdesk", "1.0.0")
            .layer(AUDIT)
            .entity::<Ticket>()
            .history_read::<Ticket>([ANALYST]),
    );
    assert!(c.contains(&"E_HISTORY_ROLE".to_string()), "{c:?}");
}

type Seen = Arc<Mutex<Vec<syscall::Call>>>;

fn platform() -> (Ctx, Seen) {
    ctx_with(move |call| match call {
        syscall::Call::Query(_) => Ok(syscall_result::Result::DbRows(pb::DbRows {
            rows: vec![bytes::Bytes::from(
                r#"{"id":"0190f3c2-0000-7000-8000-000000000101","status":"open","validFrom":"2026-11-01T10:00:00Z","validTo":null,"change":"insert","changedBy":"0190f3c2-0000-7000-8000-0000000000a1","changedByKind":"person","changedByPlugin":"helpdesk"}"#,
            )],
            ..Default::default()
        })),
        _ => Err(("unavailable: not here".into(), None)),
    })
}

#[tokio::test]
async fn a_history_read_names_the_history_and_answers_its_rows() {
    let (ctx, seen) = platform();
    let id: Id<Ticket> = Id::new("0190f3c2-0000-7000-8000-000000000101".parse().unwrap());
    let rows: Vec<HistoryRow<Ticket>> = ctx
        .lake()
        .history::<Ticket>()
        .filter(Ticket::id.eq(id))
        .order_by(history::valid_from::<Ticket>().asc())
        .all()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(
        (r.change.as_str(), r.changed_by_kind.as_str()),
        ("insert", "person")
    );
    assert!(r.valid_to.is_none() && r.day.is_none());
    assert_eq!(r.changed_by_plugin.as_deref(), Some("helpdesk"));
    assert_eq!(r.get(Ticket::status).unwrap(), Some(TicketStatus::Open));
    assert_eq!(r.get(Ticket::assignee).unwrap(), None);
    let calls = seen.lock().unwrap().clone();
    let syscall::Call::Query(q) = &calls[0] else {
        panic!("{calls:?}")
    };
    assert_eq!(q.engine, pb::Engine::Lake as i32);
    let root = &q.query.as_ref().unwrap().sources[0];
    assert_eq!(
        (root.entity.as_str(), root.history, root.each_day),
        ("ticket", true, false)
    );
    // per day, counted by day
    let (ctx, seen) = platform();
    let _ = ctx
        .lake()
        .history::<Ticket>()
        .each_day()
        .group_by(history::day::<Ticket>())
        .select((history::day::<Ticket>(), crate::query::count()))
        .all()
        .await;
    let calls = seen.lock().unwrap().clone();
    let syscall::Call::Query(q) = &calls[0] else {
        panic!("{calls:?}")
    };
    let tree = q.query.as_ref().unwrap();
    assert!(tree.sources[0].history && tree.sources[0].each_day);
    assert_eq!(tree.group_by[0].field, "day");
}

#[test]
fn read_roles_are_listed_once_each_where_first_named() {
    let p = Plugin::new("helpdesk", "1.0.0")
        .role(ANALYST)
        .role(AGENT)
        .layer(AUDIT)
        .layer(MART)
        .entity::<Ticket>()
        .history_read::<Ticket>([ANALYST, AGENT])
        .history_read::<Ticket>([ANALYST])
        .lake_table::<OpenByDay>()
        .lake_read::<OpenByDay>([AGENT, ANALYST, AGENT]);
    let json: Value = serde_json::from_str(&p.describe_json().unwrap()).unwrap();
    assert_eq!(
        json["entities"][0]["history"]["read"],
        json!(["analyst", "agent"])
    );
    assert_eq!(json["lake_tables"][0]["read"], json!(["agent", "analyst"]));
}

#[test]
fn a_model_naming_a_column_its_history_does_not_keep_is_refused_by_name() {
    #[derive(LakeTable, Clone, Debug)]
    #[lake(layer = MART, name = "titles")]
    pub struct Titles {
        pub title: Option<String>,
    }
    let m = Model::<Titles>::from_history(history::history::<Ticket>())
        .col(Titles::title, Ticket::title);
    let e = Plugin::new("helpdesk", "1.0.0")
        .role(ANALYST)
        .layer(AUDIT)
        .layer(MART)
        .entity::<Ticket>()
        .history_read_as_system::<Ticket>()
        .lake_table::<Titles>()
        .model(m)
        .describe_json()
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("E_LAKE_MODEL_COLUMN")
            && e.contains("the history of ticket does not keep \"title\""),
        "{e}"
    );
}
