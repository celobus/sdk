//! The repository of the plugin's own entities (`ctx.db::<E>()`) and declared transactions (`ctx.transaction`).
//!
//! The platform stores the rows; the SDK sends typed requests: rows in the entity's protobuf JSON, filters as a tree
//! of conditions on field handles (`Note::title.eq(..)`, combined with `&`, `|` and `!`), orders
//! (`Note::title.desc()`). A read returns [`Row<E>`]: the entity with its system columns (`created_at`, `updated_at`,
//! `deleted_at`, `version`).

use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::ops::{BitAnd, BitOr, Deref, Not};
use std::pin::Pin;

use bytes::Bytes;
use celobus_proto as pb;
use pb::{db_condition::Op, db_filter::Node, db_value::Kind, syscall, syscall_result};
use serde_json::{Map, Value};

use crate::ctx::Ctx;
use crate::error::{Error, Result};
use crate::message::{Entity, FieldValue, Message, Registry};
use crate::types::{Id, Timestamp};
use crate::ui::Field;

/// The protobuf JSON name of a field: `owner_id` → `ownerId`.
pub(crate) fn json_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    for c in name.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

// --- filters and orders ----------------------------------------------------------------------------

/// A condition on the rows of `E`: built from field handles, combined with `&`, `|`, `!`.
pub struct Filter<E> {
    node: Option<pb::DbFilter>,
    _e: PhantomData<fn() -> E>,
}

impl<E> Clone for Filter<E> {
    fn clone(&self) -> Self {
        Self::of(self.node.clone())
    }
}

impl<E> Filter<E> {
    /// The condition tree (none: every row).
    pub(crate) fn node(&self) -> Option<&pb::DbFilter> {
        self.node.as_ref()
    }

    fn of(node: Option<pb::DbFilter>) -> Self {
        Self {
            node,
            _e: PhantomData,
        }
    }

    /// Every row.
    pub fn all() -> Self {
        Self::of(None)
    }

    fn condition(field: &str, op: Op, values: Vec<pb::DbValue>) -> Self {
        Self::of(Some(pb::DbFilter {
            node: Some(Node::Condition(pb::DbCondition {
                field: field.into(),
                op: op as i32,
                values,
            })),
        }))
    }

    fn combine(self, other: Self, and: bool) -> Self {
        match (self.node, other.node) {
            (None, x) | (x, None) if and => Self::of(x),
            (None, _) | (_, None) => Self::all(),
            (Some(a), Some(b)) => {
                let list = pb::DbFilters { items: vec![a, b] };
                Self::of(Some(pb::DbFilter {
                    node: Some(if and { Node::And(list) } else { Node::Or(list) }),
                }))
            }
        }
    }
}

impl<E> BitAnd for Filter<E> {
    type Output = Filter<E>;
    fn bitand(self, rhs: Self) -> Self {
        self.combine(rhs, true)
    }
}

impl<E> BitOr for Filter<E> {
    type Output = Filter<E>;
    fn bitor(self, rhs: Self) -> Self {
        self.combine(rhs, false)
    }
}

impl<E> Not for Filter<E> {
    type Output = Filter<E>;
    fn not(self) -> Self {
        match self.node {
            // not every row: none
            None => Self::of(Some(pb::DbFilter {
                node: Some(Node::Or(pb::DbFilters { items: vec![] })),
            })),
            Some(n) => Self::of(Some(pb::DbFilter {
                node: Some(Node::Not(Box::new(n))),
            })),
        }
    }
}

/// A value as a filter parameter (the platform converts it to the column's type).
pub(crate) fn db_value(v: Value) -> pb::DbValue {
    let kind = match v {
        Value::String(s) => Some(Kind::StringValue(s)),
        Value::Bool(b) => Some(Kind::BoolValue(b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) => Some(Kind::IntValue(i)),
            None => Some(Kind::DoubleValue(n.as_f64().unwrap_or_default())),
        },
        other => Some(Kind::StringValue(other.to_string())),
    };
    pb::DbValue { kind }
}

/// A column the rows are ordered by.
pub struct Order<E> {
    field: &'static str,
    desc: bool,
    _e: PhantomData<fn() -> E>,
}

impl<E> Order<E> {
    pub(crate) fn field(&self) -> &'static str {
        self.field
    }

    pub(crate) fn is_desc(&self) -> bool {
        self.desc
    }
}

impl<E: crate::query::Queryable, T: FieldValue> Field<E, T> {
    fn compare(&self, op: Op, v: T) -> Filter<E> {
        Filter::condition(self.name(), op, vec![db_value(v.encode())])
    }

    pub fn eq(&self, v: T) -> Filter<E> {
        self.compare(Op::Eq, v)
    }

    /// Not equal (a null is not equal to a value).
    pub fn ne(&self, v: T) -> Filter<E> {
        self.compare(Op::Ne, v)
    }

    pub fn lt(&self, v: T) -> Filter<E> {
        self.compare(Op::Lt, v)
    }

    pub fn le(&self, v: T) -> Filter<E> {
        self.compare(Op::Le, v)
    }

    pub fn gt(&self, v: T) -> Filter<E> {
        self.compare(Op::Gt, v)
    }

    pub fn ge(&self, v: T) -> Filter<E> {
        self.compare(Op::Ge, v)
    }

    /// Any of the values.
    pub fn is_in(&self, values: impl IntoIterator<Item = T>) -> Filter<E> {
        Filter::condition(
            self.name(),
            Op::In,
            values.into_iter().map(|v| db_value(v.encode())).collect(),
        )
    }

    pub fn is_null(&self) -> Filter<E> {
        Filter::condition(self.name(), Op::IsNull, vec![])
    }

    pub fn is_not_null(&self) -> Filter<E> {
        Filter::condition(self.name(), Op::IsNotNull, vec![])
    }

    pub fn asc(&self) -> Order<E> {
        Order {
            field: self.name(),
            desc: false,
            _e: PhantomData,
        }
    }

    pub fn desc(&self) -> Order<E> {
        Order {
            field: self.name(),
            desc: true,
            _e: PhantomData,
        }
    }
}

/// A text field (`String`, `Option<String>`): what `search` and `contains` filter on.
pub trait Text: FieldValue {}
impl Text for String {}
impl Text for Option<String> {}

impl<E: crate::query::Queryable, T: Text> Field<E, T> {
    /// Full-text search of a `searchable` column (`#[col(searchable)]`): the rows whose text matches `words` in the
    /// column's language (web search syntax: `"a phrase"`, `or`, `-word`). The platform refuses it (400 `invalid`) on a
    /// column that is not searchable.
    pub fn search(&self, words: impl Into<String>) -> Filter<E> {
        Filter::condition(
            self.name(),
            Op::Search,
            vec![db_value(Value::String(words.into()))],
        )
    }

    /// The rows whose text has `part` in it, whatever the case; `part` is taken literally (`%` and `_` are no
    /// wildcards).
    pub fn contains(&self, part: impl Into<String>) -> Filter<E> {
        Filter::condition(
            self.name(),
            Op::Contains,
            vec![db_value(Value::String(part.into()))],
        )
    }
}

// --- navigations -----------------------------------------------------------------------------------

/// A navigation `include` brings with a row: a ref's forward navigation ([`Nav`], or the ref field's own handle), or
/// an inverse one ([`Inverse`]); `Out` is what [`Row::included`] reads of it.
pub trait Include<E> {
    type Out;
    /// The name the platform knows it by.
    fn nav_name(&self) -> &'static str;
    /// What the row's included object holds under the name (none: absent).
    fn read(value: Option<&Value>) -> Result<Self::Out>;
}

/// A ref's forward navigation (`#[col(nav = customer)]`, or the field's name without `_id`): the row it names, read
/// under that entity's own read rules.
pub struct Nav<E, T> {
    name: &'static str,
    _p: PhantomData<fn() -> (E, T)>,
}

impl<E, T> Nav<E, T> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _p: PhantomData,
        }
    }
}

impl<E, T> Clone for Nav<E, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<E, T> Copy for Nav<E, T> {}

/// An inverse navigation (`#[col(inverse = lines)]` on `OrderLine::order_id`, the handle `Order::lines`): the rows
/// of `S` whose ref names this row, read under `S`'s read rules.
pub struct Inverse<E, S> {
    name: &'static str,
    _p: PhantomData<fn() -> (E, S)>,
}

impl<E, S> Inverse<E, S> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _p: PhantomData,
        }
    }
}

impl<E, S> Clone for Inverse<E, S> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<E, S> Copy for Inverse<E, S> {}

fn included_row<T: Message + Entity>(v: &Value) -> Result<Row<T>> {
    Row::<T>::from_json(v, "")
}

impl<E, T: Message + Entity> Include<E> for Nav<E, T> {
    type Out = Option<Row<T>>;
    fn nav_name(&self) -> &'static str {
        self.name
    }
    fn read(value: Option<&Value>) -> Result<Self::Out> {
        match value {
            None | Some(Value::Null) => Ok(None),
            Some(v) => included_row(v).map(Some),
        }
    }
}

impl<E, S: Message + Entity> Include<E> for Inverse<E, S> {
    type Out = Vec<Row<S>>;
    fn nav_name(&self) -> &'static str {
        self.name
    }
    fn read(value: Option<&Value>) -> Result<Self::Out> {
        match value {
            None | Some(Value::Null) => Ok(vec![]),
            Some(Value::Array(items)) => items.iter().map(included_row).collect(),
            Some(_) => Err(Error::internal(
                "the platform sent an inverse navigation that is not a list",
            )),
        }
    }
}

/// A ref field's value: `Id<T>` or `Option<Id<T>>`, whose handle is also its forward navigation.
pub trait RefValue: FieldValue {
    type Target: Message + Entity;
}
impl<T: Message + Entity> RefValue for Id<T> {
    type Target = T;
}
impl<T: Message + Entity> RefValue for Option<Id<T>> {
    type Target = T;
}

impl<E, V: RefValue> Include<E> for Field<E, V> {
    type Out = Option<Row<V::Target>>;
    fn nav_name(&self) -> &'static str {
        self.name()
    }
    fn read(value: Option<&Value>) -> Result<Self::Out> {
        Nav::<E, V::Target>::read(value)
    }
}

// --- rows and patches ------------------------------------------------------------------------------

/// A row as reads and writes return it: the entity and its system columns. Derefs to the entity.
#[derive(Clone, Debug, PartialEq)]
pub struct Row<E> {
    value: E,
    created_at: Timestamp,
    updated_at: Timestamp,
    deleted_at: Option<Timestamp>,
    version: Option<i64>,
    /// what `include` brought with the row: navigation name → row or rows
    included: Map<String, Value>,
}

impl<E> Deref for Row<E> {
    type Target = E;
    fn deref(&self) -> &E {
        &self.value
    }
}

impl<E> Row<E> {
    pub fn into_inner(self) -> E {
        self.value
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn updated_at(&self) -> Timestamp {
        self.updated_at
    }

    /// When a soft-delete entity's row was deleted (reads `with_deleted` see such rows).
    pub fn deleted_at(&self) -> Option<Timestamp> {
        self.deleted_at
    }

    /// The row's version, on a versioned entity: what `expect_version` checks.
    pub fn version(&self) -> Option<i64> {
        self.version
    }

    /// What a read's `.include(nav)` brought with the row: for a forward navigation the row it names (`None` when
    /// there is none, the caller may not read it, or the read did not include it), for an inverse one the rows that
    /// point at this one (empty when none, or not included).
    pub fn included<I: Include<E>>(&self, nav: I) -> Result<I::Out> {
        I::read(self.included.get(nav.nav_name()))
    }
}

/// A row is its entity's message on the wire, with the system columns: an endpoint may answer one.
impl<E: Message + Entity> Message for Row<E> {
    const NAME: &'static str = E::NAME;
    fn register(reg: &mut Registry) {
        E::register(reg)
    }
    fn to_json(&self) -> Value {
        let mut v = self.value.to_json();
        if let Value::Object(m) = &mut v {
            m.insert("createdAt".into(), self.created_at.encode());
            m.insert("updatedAt".into(), self.updated_at.encode());
            if let Some(d) = &self.deleted_at {
                m.insert("deletedAt".into(), d.encode());
            }
            if let Some(n) = self.version {
                m.insert("version".into(), Value::String(n.to_string()));
            }
        }
        v
    }
    fn from_json(value: &Value, path: &str) -> Result<Self> {
        let value_of = E::from_json(value, path)?;
        let obj = crate::message::object(value, path)?;
        let time = |k: &str| -> Result<Option<Timestamp>> {
            match obj.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => Timestamp::decode(v, k).map(Some),
            }
        };
        let version = match obj.get("version") {
            None | Some(Value::Null) => None,
            Some(v) => Some(i64::decode(v, "version")?),
        };
        Ok(Row {
            value: value_of,
            created_at: time("createdAt")?.unwrap_or_default(),
            updated_at: time("updatedAt")?.unwrap_or_default(),
            deleted_at: time("deletedAt")?,
            version,
            included: Map::new(),
        })
    }
}

/// The changes of an update: fields set to a value, or cleared (`null`); the others stay.
pub struct Patch<E> {
    fields: Map<String, Value>,
    _e: PhantomData<fn() -> E>,
}

impl<E> Clone for Patch<E> {
    fn clone(&self) -> Self {
        Self {
            fields: self.fields.clone(),
            _e: PhantomData,
        }
    }
}

impl<E> Default for Patch<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> Patch<E> {
    pub fn new() -> Self {
        Self {
            fields: Map::new(),
            _e: PhantomData,
        }
    }

    pub fn set<T: FieldValue>(mut self, f: Field<E, T>, v: T) -> Self {
        self.fields.insert(json_name(f.name()), v.encode());
        self
    }

    /// Sets an optional field to null.
    pub fn clear<T: FieldValue>(mut self, f: Field<E, Option<T>>) -> Self {
        self.fields.insert(json_name(f.name()), Value::Null);
        self
    }
}

/// A page of rows and the token of the next one (none: the last page).
#[derive(Debug)]
pub struct Page<E> {
    pub rows: Vec<Row<E>>,
    pub next: Option<String>,
}

// --- the repository --------------------------------------------------------------------------------

/// The repository of an entity of the plugin: `ctx.db::<Order>()` (inside a transaction, `tx.db::<Order>()`).
pub struct Db<E> {
    ctx: Ctx,
    _e: PhantomData<fn() -> E>,
}

fn rows_of<E: Message + Entity>(
    r: Option<syscall_result::Result>,
) -> Result<(Vec<Row<E>>, String)> {
    let Some(syscall_result::Result::DbRows(r)) = r else {
        return Err(Error::internal(
            "the platform answered a db call with something else",
        ));
    };
    let json = |b: &[u8]| -> Result<Value> {
        serde_json::from_slice(b)
            .map_err(|e| Error::internal(format!("the platform sent a row that is not JSON: {e}")))
    };
    let mut rows = r
        .rows
        .iter()
        .map(|b| Row::<E>::from_json(&json(b)?, ""))
        .collect::<Result<Vec<_>>>()?;
    for (row, inc) in rows.iter_mut().zip(&r.included) {
        if let Value::Object(m) = json(inc)? {
            row.included = m;
        }
    }
    Ok((rows, r.next_page_token))
}

fn one<E: Message + Entity>(r: Option<syscall_result::Result>) -> Result<Row<E>> {
    rows_of::<E>(r)?
        .0
        .into_iter()
        .next()
        .ok_or_else(|| Error::internal("the platform answered without the row"))
}

pub(crate) fn id_text<E: Entity>(id: &Id<E>) -> String {
    match id.encode() {
        Value::String(s) => s,
        other => other.to_string(),
    }
}

impl<E: Message + Entity> Db<E> {
    pub(crate) fn new(ctx: Ctx) -> Self {
        Self {
            ctx,
            _e: PhantomData,
        }
    }

    fn request_id(&self) -> u64 {
        self.ctx.request().id()
    }

    /// A row by id; 404 `not_found` when there is none, it is soft-deleted, or the caller may not read it.
    /// `.include(nav)` brings rows it navigates to; `.await` it.
    pub fn get(&self, id: &Id<E>) -> Get<E> {
        Get {
            db: self.clone(),
            id: id_text(id),
            with_deleted: false,
            include: vec![],
            lock: None,
            _m: PhantomData,
        }
    }

    /// A row by id, a soft-deleted one too.
    pub fn get_with_deleted(&self, id: &Id<E>) -> Get<E> {
        let mut g = self.get(id);
        g.with_deleted = true;
        g
    }

    /// The rows matching `filter` (`Filter::all()` for every row): order, limit and page it, then read them with
    /// `all()` (at most 1,000), `first()` or `page()` (at most 100).
    pub fn find(&self, filter: Filter<E>) -> Find<E> {
        Find {
            db: self.clone(),
            filter,
            order: vec![],
            limit: 0,
            token: String::new(),
            with_deleted: false,
            include: vec![],
            lock: None,
            _m: PhantomData,
        }
    }

    /// Groups of the rows matching `filter` (`Filter::all()`: every row): `.group_by(E::f)` then `.count()` or
    /// `.sum(E::g)` → `Vec<(key, value)>` (at most 1,000 groups, else 422 `query_rows_limit`), or `.count()` /
    /// `.sum(E::g)` of them all. Read under the entity's read rules, as `find` is.
    pub fn aggregate(&self, filter: Filter<E>) -> Aggregate<E> {
        Aggregate {
            db: self.clone(),
            filter,
            with_deleted: false,
        }
    }

    /// Changes the fields `patch` names on every row matching `filter` (`Filter::all()`: every row the caller may
    /// read), in one statement; the rows changed. Each row passes the `update` rule and the guards as `update`'s
    /// does (one that does not refuses them all: 403); more than 1,000 rows is 422 `bulk_limit`, and nothing is
    /// written.
    pub async fn update_where(&self, filter: Filter<E>, patch: Patch<E>) -> Result<u64> {
        let patch = Bytes::from(serde_json::to_vec(&Value::Object(patch.fields))?);
        let r = self
            .ctx
            .syscall(syscall::Call::DbUpdateWhere(pb::DbUpdateWhere {
                request_id: self.request_id(),
                as_system: self.ctx.system(),
                tx: self.ctx.tx(),
                entity: E::ENTITY.into(),
                filter: filter.node,
                patch,
            }))
            .await?;
        match r {
            Some(syscall_result::Result::DbCount(c)) => Ok(c.count),
            _ => Err(Error::internal(
                "the platform answered an update_where with something else",
            )),
        }
    }

    /// Creates `value`, or changes the row that has its value of the unique column `on` to it (on a soft-delete
    /// entity, a row that is not deleted): `on` is a `#[col(unique)]` field, or `E::id` of a `string`-key entity.
    /// The create rule applies to a new row, the update rule and the guards to a changed one.
    pub async fn upsert<T: FieldValue>(&self, on: Field<E, T>, value: E) -> Result<Row<E>> {
        let row = Bytes::from(serde_json::to_vec(&value.to_json())?);
        let r = self
            .ctx
            .syscall(syscall::Call::DbUpsert(pb::DbUpsert {
                request_id: self.request_id(),
                as_system: self.ctx.system(),
                tx: self.ctx.tx(),
                entity: E::ENTITY.into(),
                on: on.name().into(),
                row,
            }))
            .await?;
        one(r)
    }

    /// Every row (at most 1,000).
    pub async fn all(&self) -> Result<Vec<Row<E>>> {
        self.find(Filter::all()).all().await
    }

    pub async fn count(&self) -> Result<u64> {
        self.count_where(Filter::all()).await
    }

    pub async fn count_where(&self, filter: Filter<E>) -> Result<u64> {
        self.count_rows(filter, false).await
    }

    async fn count_rows(&self, filter: Filter<E>, with_deleted: bool) -> Result<u64> {
        let r = self
            .ctx
            .syscall(syscall::Call::DbCount(pb::DbCount {
                request_id: self.request_id(),
                as_system: self.ctx.system(),
                tx: self.ctx.tx(),
                entity: E::ENTITY.into(),
                filter: filter.node,
                with_deleted,
                dependency: E::PLUGIN.unwrap_or_default().into(),
            }))
            .await?;
        match r {
            Some(syscall_result::Result::DbCount(c)) => Ok(c.count),
            _ => Err(Error::internal(
                "the platform answered a count with something else",
            )),
        }
    }

    pub async fn exists(&self, filter: Filter<E>) -> Result<bool> {
        Ok(self.find(filter).first().await?.is_some())
    }

    /// A new row: the store makes a `uuid` or `i64` key (the value's `id` is not read: `Id::none()`); a `string`
    /// key is the value's.
    pub async fn create(&self, value: E) -> Result<Row<E>> {
        self.create_many(vec![value])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| Error::internal("the platform answered a create without the row"))
    }

    /// New rows, in one statement (at most 1,000).
    pub async fn create_many(&self, values: Vec<E>) -> Result<Vec<Row<E>>> {
        let rows = values
            .iter()
            .map(|v| Bytes::from(serde_json::to_vec(&v.to_json()).unwrap_or_default()))
            .collect();
        let r = self
            .ctx
            .syscall(syscall::Call::DbCreate(pb::DbCreate {
                request_id: self.request_id(),
                as_system: self.ctx.system(),
                tx: self.ctx.tx(),
                entity: E::ENTITY.into(),
                rows,
            }))
            .await?;
        Ok(rows_of(r)?.0)
    }

    /// Changes a row's fields; `.expect_version(v)` makes it `version_conflict` when the row is at another version.
    pub fn update(&self, id: &Id<E>, patch: Patch<E>) -> Update<E> {
        Update {
            db: self.clone(),
            id: id_text(id),
            patch,
            expected: None,
        }
    }

    /// Deletes a row (marks it, on a soft-delete entity); `.expect_version(v)` checks its version.
    pub fn delete(&self, id: &Id<E>) -> Delete<E> {
        Delete {
            db: self.clone(),
            id: id_text(id),
            expected: None,
            force: false,
        }
    }

    /// Removes a row of a soft-delete entity for good.
    pub fn delete_force(&self, id: &Id<E>) -> Delete<E> {
        Delete {
            db: self.clone(),
            id: id_text(id),
            expected: None,
            force: true,
        }
    }

    /// Brings a soft-deleted row back.
    pub async fn restore(&self, id: &Id<E>) -> Result<Row<E>> {
        let r = self
            .ctx
            .syscall(syscall::Call::DbRestore(pb::DbRestore {
                request_id: self.request_id(),
                as_system: self.ctx.system(),
                tx: self.ctx.tx(),
                entity: E::ENTITY.into(),
                id: id_text(id),
            }))
            .await?;
        one(r)
    }
}

impl<E> Clone for Db<E> {
    fn clone(&self) -> Self {
        Self {
            ctx: self.ctx.clone(),
            _e: PhantomData,
        }
    }
}

/// A read outside a transaction, which takes no row locks.
pub struct Plain;
/// A read of `tx.db::<E>()` in a declared transaction, which may lock the rows it reads.
pub struct Locking;

fn lock(strength: pb::db_lock::Strength) -> Option<pb::DbLock> {
    Some(pb::DbLock {
        strength: strength as i32,
        wait: pb::db_lock::Wait::Wait as i32,
    })
}

fn lock_wait(l: &mut Option<pb::DbLock>, wait: pb::db_lock::Wait) {
    let l = l.get_or_insert(pb::DbLock {
        strength: pb::db_lock::Strength::ForUpdate as i32,
        wait: 0,
    });
    l.wait = wait as i32;
}

/// A query being built: `db.find(filter).order_by(..).limit(..)`, read with `all()`, `first()` or `page()`. In a
/// transaction (`tx.db::<E>().find(..)`) it may lock the rows it reads: `.for_update()`, `.for_share()`.
pub struct Find<E, M = Plain> {
    db: Db<E>,
    filter: Filter<E>,
    order: Vec<pb::DbOrder>,
    limit: u32,
    token: String,
    with_deleted: bool,
    include: Vec<String>,
    lock: Option<pb::DbLock>,
    _m: PhantomData<M>,
}

impl<E: Message + Entity> Find<E, Locking> {
    /// Locks the rows read until the transaction ends, against changes and other locks (`SELECT .. FOR UPDATE`);
    /// waiting for a row another transaction holds counts against the transaction's time.
    pub fn for_update(mut self) -> Self {
        self.lock = lock(pb::db_lock::Strength::ForUpdate);
        self
    }

    /// Locks the rows read against changes until the transaction ends; others may share the lock.
    pub fn for_share(mut self) -> Self {
        self.lock = lock(pb::db_lock::Strength::ForShare);
        self
    }

    /// Leaves out the rows another transaction holds (a work queue: concurrent runs take different rows). Locks
    /// for update unless `for_share` says otherwise.
    pub fn skip_locked(mut self) -> Self {
        lock_wait(&mut self.lock, pb::db_lock::Wait::SkipLocked);
        self
    }

    /// Fails at once (409 `row_locked`) instead of waiting for a row another transaction holds.
    pub fn nowait(mut self) -> Self {
        lock_wait(&mut self.lock, pb::db_lock::Wait::Nowait);
        self
    }
}

impl<E: Message + Entity, M> Find<E, M> {
    /// Brings the rows a navigation leads to with each row (`Order::customer`, `Order::lines`), read under their
    /// entity's own rules: [`Row::included`] reads them. They count toward the 1,000 rows of a read.
    pub fn include(mut self, nav: impl Include<E>) -> Self {
        self.include.push(nav.nav_name().into());
        self
    }

    /// Orders by a column; called again, by the next one (ties end by id).
    pub fn order_by(mut self, o: Order<E>) -> Self {
        self.order.push(pb::DbOrder {
            field: o.field.into(),
            desc: o.desc,
        });
        self
    }

    /// Rows per page (100 at most; 100 by default).
    pub fn limit(mut self, n: u32) -> Self {
        self.limit = n;
        self
    }

    /// The page after the one whose `next` this is.
    pub fn page_token(mut self, token: &str) -> Self {
        self.token = token.into();
        self
    }

    /// Soft-deleted rows too.
    pub fn with_deleted(mut self) -> Self {
        self.with_deleted = true;
        self
    }

    fn request(&self, all: bool, limit: u32) -> syscall::Call {
        syscall::Call::DbFind(pb::DbFind {
            request_id: self.db.request_id(),
            as_system: self.db.ctx.system(),
            tx: self.db.ctx.tx(),
            entity: E::ENTITY.into(),
            filter: self.filter.node.clone(),
            order: self.order.clone(),
            limit,
            page_token: self.token.clone(),
            with_deleted: self.with_deleted,
            all,
            dependency: E::PLUGIN.unwrap_or_default().into(),
            include: self.include.clone(),
            lock: self.lock,
        })
    }

    /// Every matching row: more than 1,000 is `query_rows_limit` (page through them instead).
    pub async fn all(self) -> Result<Vec<Row<E>>> {
        let r = self.db.ctx.syscall(self.request(true, 0)).await?;
        Ok(rows_of(r)?.0)
    }

    pub async fn first(self) -> Result<Option<Row<E>>> {
        let r = self.db.ctx.syscall(self.request(false, 1)).await?;
        Ok(rows_of(r)?.0.into_iter().next())
    }

    pub async fn page(self) -> Result<Page<E>> {
        let r = self.db.ctx.syscall(self.request(false, self.limit)).await?;
        let (rows, next) = rows_of(r)?;
        Ok(Page {
            rows,
            next: (!next.is_empty()).then_some(next),
        })
    }
}

type Boxed<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// A read of one row being built: `db.get(&id)`, with `.include(nav)`; in a transaction (`tx.db::<E>().get(..)`)
/// `.for_update()`, `.for_share()`, `.nowait()`. `.await` it.
pub struct Get<E, M = Plain> {
    db: Db<E>,
    id: String,
    with_deleted: bool,
    include: Vec<String>,
    lock: Option<pb::DbLock>,
    _m: PhantomData<M>,
}

impl<E: Message + Entity, M> Get<E, M> {
    /// Brings the rows a navigation leads to with the row ([`Row::included`] reads them).
    pub fn include(mut self, nav: impl Include<E>) -> Self {
        self.include.push(nav.nav_name().into());
        self
    }
}

impl<E: Message + Entity> Get<E, Locking> {
    /// Locks the row until the transaction ends (`FOR UPDATE`).
    pub fn for_update(mut self) -> Self {
        self.lock = lock(pb::db_lock::Strength::ForUpdate);
        self
    }

    /// Locks the row against changes until the transaction ends; others may share the lock (`FOR SHARE`).
    pub fn for_share(mut self) -> Self {
        self.lock = lock(pb::db_lock::Strength::ForShare);
        self
    }

    /// Fails at once (409 `row_locked`) instead of waiting when another transaction holds the row.
    pub fn nowait(mut self) -> Self {
        lock_wait(&mut self.lock, pb::db_lock::Wait::Nowait);
        self
    }
}

impl<E: Message + Entity, M: Send + 'static> IntoFuture for Get<E, M> {
    type Output = Result<Row<E>>;
    type IntoFuture = Boxed<'static, Row<E>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let r = self
                .db
                .ctx
                .syscall(syscall::Call::DbGet(pb::DbGet {
                    request_id: self.db.request_id(),
                    as_system: self.db.ctx.system(),
                    tx: self.db.ctx.tx(),
                    entity: E::ENTITY.into(),
                    id: self.id,
                    with_deleted: self.with_deleted,
                    dependency: E::PLUGIN.unwrap_or_default().into(),
                    include: self.include,
                    lock: self.lock,
                }))
                .await?;
            one(r)
        })
    }
}

/// A column that sums: `i32` and `i64` sum to an `i64`, `f64` to an `f64`, `Decimal` to a `Decimal` (an optional
/// column as its type; nulls count as nothing).
pub trait Summable: FieldValue {
    type Sum: FieldValue;
}
impl Summable for i32 {
    type Sum = i64;
}
impl Summable for i64 {
    type Sum = i64;
}
impl Summable for f64 {
    type Sum = f64;
}
impl Summable for crate::types::Decimal {
    type Sum = crate::types::Decimal;
}
impl<T: Summable> Summable for Option<T> {
    type Sum = T::Sum;
}

/// An aggregate being built: `db.aggregate(filter)`, then `.group_by(E::f)` or not, then `.count()` or `.sum(E::g)`.
pub struct Aggregate<E> {
    db: Db<E>,
    filter: Filter<E>,
    with_deleted: bool,
}

/// An aggregate by the values of one column: `.count()` or `.sum(E::g)` of each group, in key order (nulls last).
pub struct Grouped<E, K> {
    aggregate: Aggregate<E>,
    key: &'static str,
    _k: PhantomData<fn() -> K>,
}

impl<E: Message + Entity> Aggregate<E> {
    /// Soft-deleted rows too.
    pub fn with_deleted(mut self) -> Self {
        self.with_deleted = true;
        self
    }

    /// Groups by a column's values (a field or a system column).
    pub fn group_by<K: FieldValue>(self, f: Field<E, K>) -> Grouped<E, K> {
        Grouped {
            aggregate: self,
            key: f.name(),
            _k: PhantomData,
        }
    }

    async fn groups(
        &self,
        group_by: &str,
        measure: pb::db_aggregate::Measure,
        field: &str,
    ) -> Result<Vec<(Value, Value)>> {
        let r = self
            .db
            .ctx
            .syscall(syscall::Call::DbAggregate(pb::DbAggregate {
                request_id: self.db.request_id(),
                as_system: self.db.ctx.system(),
                tx: self.db.ctx.tx(),
                entity: E::ENTITY.into(),
                filter: self.filter.node.clone(),
                with_deleted: self.with_deleted,
                dependency: E::PLUGIN.unwrap_or_default().into(),
                group_by: group_by.into(),
                measure: measure as i32,
                field: field.into(),
            }))
            .await?;
        let Some(syscall_result::Result::DbGroups(g)) = r else {
            return Err(Error::internal(
                "the platform answered an aggregate with something else",
            ));
        };
        let json = |b: &[u8]| -> Result<Value> {
            serde_json::from_slice(b).map_err(|e| {
                Error::internal(format!("the platform sent a group that is not JSON: {e}"))
            })
        };
        g.groups
            .iter()
            .map(|x| Ok((json(&x.key)?, json(&x.value)?)))
            .collect()
    }

    /// The rows matching the filter (as `count_where`).
    pub async fn count(self) -> Result<u64> {
        let groups = self
            .groups("", pb::db_aggregate::Measure::Count, "")
            .await?;
        match groups.first() {
            Some((_, v)) => Ok(i64::decode(v, "count")? as u64),
            None => Ok(0),
        }
    }

    /// The sum of a column over the rows matching the filter (0 when none).
    pub async fn sum<T: Summable>(self, f: Field<E, T>) -> Result<T::Sum> {
        let groups = self
            .groups("", pb::db_aggregate::Measure::Sum, f.name())
            .await?;
        match groups.first() {
            Some((_, v)) => T::Sum::decode(v, "sum"),
            None => T::Sum::missing("sum"),
        }
    }
}

impl<E: Message + Entity, K: FieldValue> Grouped<E, K> {
    /// Each group's key and its number of rows.
    pub async fn count(self) -> Result<Vec<(K, u64)>> {
        self.aggregate
            .groups(self.key, pb::db_aggregate::Measure::Count, "")
            .await?
            .iter()
            .map(|(k, v)| Ok((K::decode(k, "key")?, i64::decode(v, "count")? as u64)))
            .collect()
    }

    /// Each group's key and the sum of a column over its rows.
    pub async fn sum<T: Summable>(self, f: Field<E, T>) -> Result<Vec<(K, T::Sum)>> {
        self.aggregate
            .groups(self.key, pb::db_aggregate::Measure::Sum, f.name())
            .await?
            .iter()
            .map(|(k, v)| Ok((K::decode(k, "key")?, T::Sum::decode(v, "sum")?)))
            .collect()
    }
}

/// An update being built; `.await` it.
pub struct Update<E> {
    db: Db<E>,
    id: String,
    patch: Patch<E>,
    expected: Option<i64>,
}

impl<E> Update<E> {
    /// Refuses the update (`version_conflict`) unless the row is at this version.
    pub fn expect_version(mut self, v: impl Into<Option<i64>>) -> Self {
        self.expected = v.into();
        self
    }
}

impl<E: Message + Entity> IntoFuture for Update<E> {
    type Output = Result<Row<E>>;
    type IntoFuture = Boxed<'static, Row<E>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let patch = Bytes::from(serde_json::to_vec(&Value::Object(self.patch.fields))?);
            let r = self
                .db
                .ctx
                .syscall(syscall::Call::DbUpdate(pb::DbUpdate {
                    request_id: self.db.request_id(),
                    as_system: self.db.ctx.system(),
                    tx: self.db.ctx.tx(),
                    entity: E::ENTITY.into(),
                    id: self.id,
                    patch,
                    expected_version: self.expected,
                }))
                .await?;
            one(r)
        })
    }
}

/// A delete being built; `.await` it.
pub struct Delete<E> {
    db: Db<E>,
    id: String,
    expected: Option<i64>,
    force: bool,
}

impl<E> Delete<E> {
    pub fn expect_version(mut self, v: impl Into<Option<i64>>) -> Self {
        self.expected = v.into();
        self
    }
}

impl<E: Message + Entity> IntoFuture for Delete<E> {
    type Output = Result<()>;
    type IntoFuture = Boxed<'static, ()>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            self.db
                .ctx
                .syscall(syscall::Call::DbDelete(pb::DbDelete {
                    request_id: self.db.request_id(),
                    as_system: self.db.ctx.system(),
                    tx: self.db.ctx.tx(),
                    entity: E::ENTITY.into(),
                    id: self.id,
                    expected_version: self.expected,
                    force: self.force,
                }))
                .await
                .map(|_| ())
        })
    }
}

// --- transactions ----------------------------------------------------------------------------------

/// How a declared transaction runs: read committed by default; not for a nested block (it runs in its parent).
#[derive(Clone, Copy, Debug, Default)]
pub struct TxOptions {
    isolation: pb::tx_begin::Isolation,
    read_only: bool,
}

impl TxOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn repeatable_read(mut self) -> Self {
        self.isolation = pb::tx_begin::Isolation::RepeatableRead;
        self
    }

    pub fn serializable(mut self) -> Self {
        self.isolation = pb::tx_begin::Isolation::Serializable;
        self
    }

    /// Refuses writes.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }
}

/// The context of a declared transaction (`ctx.transaction(async |tx| ..)`): a [`Ctx`] (it derefs to one) whose
/// repository calls are part of the transaction, and whose reads may lock rows (`tx.db::<E>().get(&id)
/// .for_update()`); row locks exist only here.
///
/// ```
/// # use celobus_sdk::prelude::*;
/// # #[derive(Entity, Clone, Debug)] pub struct Job { pub id: Id<Job>, pub done: bool }
/// async fn take(ctx: Ctx) -> Result<Option<Row<Job>>> {
///     ctx.transaction(async |tx| {
///         tx.db::<Job>().find(Job::done.eq(false)).for_update().skip_locked().first().await
///     })
///     .await
/// }
/// ```
///
/// Outside a transaction a read has no lock methods:
///
/// ```compile_fail
/// # use celobus_sdk::prelude::*;
/// # #[derive(Entity, Clone, Debug)] pub struct Job { pub id: Id<Job>, pub done: bool }
/// async fn take(ctx: Ctx, id: Id<Job>) -> Result<Row<Job>> {
///     ctx.db::<Job>().get(&id).for_update().await
/// }
/// ```
#[derive(Clone)]
pub struct Tx {
    ctx: Ctx,
}

impl Deref for Tx {
    type Target = Ctx;
    fn deref(&self) -> &Ctx {
        &self.ctx
    }
}

impl Tx {
    /// The repository of one of the plugin's entities in this transaction, with row locks.
    pub fn db<E: Message + Entity>(&self) -> TxDb<E> {
        TxDb {
            db: self.ctx.db::<E>(),
        }
    }

    /// The transaction's context as a plain [`Ctx`] (its calls are still part of the transaction).
    pub fn ctx(&self) -> &Ctx {
        &self.ctx
    }

    /// The same transaction with the system's rights (see [`Ctx::as_system`]).
    pub fn as_system(&self, reason: &str) -> Tx {
        Tx {
            ctx: self.ctx.as_system(reason),
        }
    }
}

/// The repository of an entity in a declared transaction: [`Db`] (it derefs to it), and reads that lock rows.
pub struct TxDb<E> {
    db: Db<E>,
}

impl<E> Deref for TxDb<E> {
    type Target = Db<E>;
    fn deref(&self) -> &Db<E> {
        &self.db
    }
}

impl<E: Message + Entity> TxDb<E> {
    /// A row by id, as [`Db::get`]; `.for_update()`, `.for_share()` and `.nowait()` lock it.
    pub fn get(&self, id: &Id<E>) -> Get<E, Locking> {
        let g = self.db.get(id);
        Get {
            db: g.db,
            id: g.id,
            with_deleted: g.with_deleted,
            include: g.include,
            lock: None,
            _m: PhantomData,
        }
    }

    /// The rows matching `filter`, as [`Db::find`]; `.for_update()`, `.for_share()`, `.skip_locked()` and
    /// `.nowait()` lock them.
    pub fn find(&self, filter: Filter<E>) -> Find<E, Locking> {
        let f = self.db.find(filter);
        Find {
            db: f.db,
            filter: f.filter,
            order: f.order,
            limit: f.limit,
            token: f.token,
            with_deleted: f.with_deleted,
            include: f.include,
            lock: None,
            _m: PhantomData,
        }
    }
}

impl Ctx {
    /// The repository of one of the plugin's entities; inside a transaction (`tx.db::<E>()`) its calls are part of it.
    pub fn db<E: Message + Entity>(&self) -> Db<E> {
        const {
            assert!(
                E::PLUGIN.is_none(),
                "ctx.db is the plugin's own entities: read a dependency's with ctx.query"
            )
        };
        const {
            assert!(
                !E::OLAP,
                "an entity stored in OLAP is neither read nor written with ctx.db: its load fills it, \
                 ctx.analytics().query() reads it"
            )
        };
        Db::new(self.clone())
    }

    /// A declared transaction: `f` gets `tx`, a context whose repository calls are part of it; it commits when `f`
    /// returns `Ok`, rolls back otherwise. Inside a block it rolls back alone. The platform rolls back a transaction
    /// still open when the request ends or 5 s pass (`transaction_timeout`).
    pub async fn transaction<T, F, Fut>(&self, f: F) -> Result<T>
    where
        F: FnOnce(Tx) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        self.transaction_with(TxOptions::default(), f).await
    }

    /// [`Ctx::transaction`] with an isolation level or read-only.
    pub async fn transaction_with<T, F, Fut>(&self, options: TxOptions, f: F) -> Result<T>
    where
        F: FnOnce(Tx) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let r = self
            .syscall(syscall::Call::TxBegin(pb::TxBegin {
                request_id: self.request().id(),
                tx: self.tx(),
                isolation: options.isolation as i32,
                read_only: options.read_only,
                as_system: self.system(),
            }))
            .await?;
        let Some(syscall_result::Result::TxBegun(b)) = r else {
            return Err(Error::internal(
                "the platform answered a begin with something else",
            ));
        };
        let end = |commit: bool| {
            let request_id = self.request().id();
            let call = if commit {
                syscall::Call::TxCommit(pb::TxCommit {
                    request_id,
                    tx: b.tx,
                })
            } else {
                syscall::Call::TxRollback(pb::TxRollback {
                    request_id,
                    tx: b.tx,
                })
            };
            self.syscall(call)
        };
        match f(Tx {
            ctx: self.in_transaction(b.tx),
        })
        .await
        {
            Ok(v) => {
                end(true).await?;
                Ok(v)
            }
            Err(e) => {
                let _ = end(false).await;
                Err(e)
            }
        }
    }
}

impl<E: Entity> Id<E>
where
    E::Key: Default,
{
    /// The id of a value not stored yet: the store makes a `uuid` or `i64` key on `create`.
    pub fn none() -> Self {
        Id::new(E::Key::default())
    }
}
