//! An entity's history in the lake: every change of the columns the entity keeps, each its own row with when it was
//! made and by whom, kept by the platform without a time limit in a layer the plugin declares.
//!
//! ```ignore
//! pub const AUDIT: Layer = Layer::new("audit").title("History of records");
//!
//! #[derive(Entity, Clone, Debug)]
//! #[entity(soft_delete, history(layer = AUDIT))]
//! pub struct Ticket {
//!     pub id: Id<Ticket>,
//!     pub title: String,
//!     #[col(history)] pub status: TicketStatus,
//!     #[col(history)] pub assignee: Option<Id<platform::User>>,
//! }
//! // Plugin::new(..).layer(AUDIT).entity::<Ticket>().history_read::<Ticket>([ANALYST])
//!
//! let trail = ctx.lake().history::<Ticket>()
//!     .filter(Ticket::id.eq(id))
//!     .order_by(history::valid_from::<Ticket>().asc())
//!     .all()
//!     .await?;                                                 // Vec<HistoryRow<Ticket>>
//! ```
//!
//! **What is kept.** The columns marked `#[col(history)]`, or with `history(layer = .., all)` every column (the ones a
//! later version adds too) but the encrypted ones and, unless `unmasked` is also said, the masked ones (the lake has
//! no masks; `celobus plugin check` lists what `all` left out). A kept column is a scalar, an enum or a ref.
//!
//! **A row** is a change: the entity's `id`, the kept columns as they were after it, [`valid_from`] (when it was
//! made), [`valid_to`] (the row's next change; none for the current version; equal to `valid_from` on a delete),
//! [`change`] (`baseline`, `insert`, `update`, `delete`, `restore`) and who made it: [`changed_by`] (the person or
//! service account, none for a system context or a job without one) and [`changed_by_kind`] (`person`, `service`,
//! `job`, `system`, `platform`, `anonymous`) and [`changed_by_plugin`] (the plugin whose call made it). Two changes are never merged: two updates of a row in one transaction
//! are two rows. An update that changes no kept column is none; a soft delete is a `delete`, and undoing it a
//! `restore`. When the history begins (the plugin's install, or an upgrade that keeps a new column), every row is
//! written as it is then, as `baseline`. Changes reach the lake within seconds, in order.
//!
//! **Per day.** `.each_day()` reads one row per day each version was in force at the day's last moment, in the
//! workspace's time zone, with its [`day`]: from the day of `valid_from` to the day before that of `valid_to`, to today
//! for the current version (a version that began and ended within one day, and a delete, have none). So counting the
//! rows of a day counts what was there at its end (scenario: open tickets per day).
//!
//! **Who reads it.** The holders of the roles `Plugin::history_read` names, every row; the plugin's own system
//! context only with `Plugin::history_read_as_system`; another plugin only when the owner exports it
//! (`Plugin::history_export`) and that plugin declares it (`Plugin::history_dependency`). A history is read in the
//! lake only, with the lake's limits; a model reads it too ([`super::Model::from_history`]).

use std::marker::PhantomData;

use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    message::{Entity, FieldValue},
    query::{QueryRow, Queryable},
    types::{Date, Id, Timestamp},
    ui::Field,
};

/// The history of `E` as a query's root: its rows are [`HistoryRow<E>`].
pub struct History<E>(PhantomData<fn() -> E>);

impl<E: Entity + Queryable> Queryable for History<E> {
    const SOURCE: &'static str = E::SOURCE;
    const OWNER: Option<&'static str> = E::OWNER;
}

impl<E: Entity + Queryable> QueryRow for History<E> {
    type Row = HistoryRow<E>;
    fn row(v: &Value) -> Result<HistoryRow<E>> {
        HistoryRow::from_json(v)
    }
}

/// A change of a row of `E`.
#[derive(Debug, Clone)]
pub struct HistoryRow<E: Entity> {
    pub id: Id<E>,
    pub valid_from: Timestamp,
    pub valid_to: Option<Timestamp>,
    /// `baseline`, `insert`, `update`, `delete`, `restore`
    pub change: String,
    pub changed_by: Option<Id<crate::platform::User>>,
    /// `person`, `service`, `job`, `system`, `platform`, `anonymous`
    pub changed_by_kind: String,
    /// the plugin whose call made it (this plugin's, or another's that wrote its rows; none for the platform's own)
    pub changed_by_plugin: Option<String>,
    /// read per day: the day
    pub day: Option<Date>,
    values: Map<String, Value>,
}

impl<E: Entity> HistoryRow<E> {
    fn from_json(v: &Value) -> Result<Self> {
        let obj = v
            .as_object()
            .ok_or_else(|| Error::invalid("a history row is an object"))?;
        let get = |k: &str| obj.get(k).unwrap_or(&Value::Null);
        Ok(HistoryRow {
            id: FieldValue::decode(get("id"), "id")?,
            valid_from: FieldValue::decode(get("validFrom"), "validFrom")?,
            valid_to: match get("validTo") {
                Value::Null => None,
                x => Some(FieldValue::decode(x, "validTo")?),
            },
            change: FieldValue::decode(get("change"), "change")?,
            changed_by: match get("changedBy") {
                Value::Null => None,
                x => Some(FieldValue::decode(x, "changedBy")?),
            },
            changed_by_kind: FieldValue::decode(get("changedByKind"), "changedByKind")?,
            changed_by_plugin: match get("changedByPlugin") {
                Value::Null => None,
                x => Some(FieldValue::decode(x, "changedByPlugin")?),
            },
            day: match get("day") {
                Value::Null => None,
                x => Some(FieldValue::decode(x, "day")?),
            },
            values: obj.clone(),
        })
    }

    /// A kept column as it was after the change (`None`: the row has no value of it, as when it was kept later).
    pub fn get<V: FieldValue>(&self, f: Field<E, V>) -> Result<Option<V>> {
        let key = crate::db::json_name(f.name());
        match self.values.get(&key) {
            None | Some(Value::Null) => Ok(None),
            Some(x) => V::decode(x, &key).map(Some),
        }
    }
}

/// When a change was made.
pub const fn valid_from<E>() -> Field<E, Timestamp> {
    Field::new("valid_from")
}

/// The row's next change (none: the current version; on a delete, the delete's own moment).
pub const fn valid_to<E>() -> Field<E, Option<Timestamp>> {
    Field::new("valid_to")
}

/// `baseline`, `insert`, `update`, `delete`, `restore`.
pub const fn change<E>() -> Field<E, String> {
    Field::new("change")
}

/// Who made it: a person or a service account (none for a system context, or a job without one).
pub const fn changed_by<E>() -> Field<E, Option<Id<crate::platform::User>>> {
    Field::new("changed_by")
}

/// `person`, `service`, `job`, `system`, `platform`, `anonymous`.
pub const fn changed_by_kind<E>() -> Field<E, String> {
    Field::new("changed_by_kind")
}

/// The plugin whose call made it (none for the platform's own).
pub const fn changed_by_plugin<E>() -> Field<E, Option<String>> {
    Field::new("changed_by_plugin")
}

/// The day of a history read per day (`each_day()`).
pub const fn day<E>() -> Field<E, Date> {
    Field::new("day")
}

/// The history of `E` as a model's root (`Model::from_history(history::<Ticket>().each_day())`).
pub struct Source<E> {
    pub(crate) each_day: bool,
    _e: PhantomData<fn() -> E>,
}

/// The history of `E`, per change.
pub fn history<E: Entity + Queryable>() -> Source<E> {
    Source {
        each_day: false,
        _e: PhantomData,
    }
}

impl<E> Source<E> {
    /// One row per day each version was in force at the day's end, with its [`day`].
    pub fn each_day(mut self) -> Self {
        self.each_day = true;
        self
    }
}
