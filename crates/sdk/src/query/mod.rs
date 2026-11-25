//! Typed reads across entities: `ctx.query::<E>()` joins, filters, groups and aggregates over the plugin's own
//! entities, its dependencies' exported entities and the workspace directory, typed end to end. The SDK builds a tree
//! of entities and columns by their names, operators and values (`celobus.v1.Query`), never SQL; the platform checks it
//! against the installed plugins' declarations and runs it read-only as the caller, every
//! entity under its own rules (a dependency's by its provider's exported `read`, the directory by the reader's base
//! role).
//!
//! ```ignore
//! use celobus_sdk::query::{label, sum};
//! let top = ctx.query::<OrderLine>()
//!     .join::<Order>(OrderLine::order_id)                         // a ref: order_line → order
//!     .join::<customers::Customer>(Order::customer_id)            // a dependency's exported entity
//!     .filter(Order::status.eq(OrderStatus::Paid) & Order::paid_at.ge(month_start))
//!     .group_by(customers::Customer::id)
//!     .select((customers::Customer::name, label(customers::Customer::account_manager_id), sum(OrderLine::line_total)))
//!     .order_by(sum(OrderLine::line_total).desc())
//!     .limit(20)
//!     .all()
//!     .await?;                                                    // Vec<(String, Option<Label>, Decimal)>
//! ```
//!
//! The same builder reads the analytic copy of the workspace's entities: `ctx.analytics().query::<E>()` takes the same
//! joins, filters, groups and aggregates under the same access rules and masks, and answers what `ctx.query()` would
//! answer for the data the copy holds. The copy follows the data, usually within a second, never with a promise: a
//! caller does not see their own write in it at once, so a screen that shows what was just saved reads it with
//! `ctx.query()`. Every analytic answer says when its data is from (`as_of`); it is never waited for and never
//! refused for being old. A query runs at most 10 s (`analytic_time_limit`) within the copy's memory
//! (`analytic_memory_limit`); four of the workspace run at once, two of one plugin (`analytic_busy` after a short
//! wait); `unavailable` while the copy is being made, when its data is older than the workspace's limit (15 minutes
//! unless the workspace sets another; never a fallback to `ctx.query()`) or when the workspace has none. It is refused
//! inside a transaction; `search` is not
//! available there. Text compares and orders by its bytes there. Decimals compute there with at most 20 digits before
//! the point and 18 after: a query that compares, orders, groups or aggregates a column holding a longer value the
//! caller may read (or binds one, or sums past them) is `query_invalid`, naming the column, never rounded. A group by
//! a decimal, and its `min` or `max`, answer the shortest text of the number (`1.5` for `1.5` and `1.50`).
//!
//! ```ignore
//! let sales = ctx.analytics().query::<OrderLine>()
//!     .join::<Order>(OrderLine::order_id)
//!     .group_by(Order::status)
//!     .select((Order::status, sum(OrderLine::line_total)))
//!     .all_as_of()
//!     .await?;                                                    // sales.rows, sales.as_of
//! ```
//!
//! A join on a ref between other entities, a column of an entity the query does not have, a comparison of columns of
//! different types do not compile:
//!
//! ```compile_fail
//! # use celobus_sdk::prelude::*;
//! # #[derive(Entity, Clone, Debug)] pub struct Customer { pub id: Id<Customer>, pub name: String }
//! # #[derive(Entity, Clone, Debug)] pub struct Product { pub id: Id<Product>, pub name: String }
//! # #[derive(Entity, Clone, Debug)] pub struct Order { pub id: Id<Order>, pub customer_id: Id<Customer> }
//! # fn f(ctx: Ctx) {
//! // `Id<Customer>` is not a ref to a product
//! ctx.query::<Order>().join::<Product>(Order::customer_id);
//! # }
//! ```
//!
//! ```compile_fail
//! # use celobus_sdk::prelude::*;
//! # #[derive(Entity, Clone, Debug)] pub struct Customer { pub id: Id<Customer>, pub name: String }
//! # #[derive(Entity, Clone, Debug)] pub struct Order { pub id: Id<Order>, pub customer_id: Id<Customer> }
//! # fn f(ctx: Ctx) {
//! // the query has no customer
//! ctx.query::<Order>().select((Order::id, Customer::name));
//! # }
//! ```
//!
//! ```compile_fail
//! # use celobus_sdk::prelude::*;
//! # use celobus_sdk::query::ColumnCompare;
//! # #[derive(Entity, Clone, Debug)] pub struct Customer { pub id: Id<Customer>, pub name: String }
//! # #[derive(Entity, Clone, Debug)] pub struct Order { pub id: Id<Order>, pub customer_id: Id<Customer> }
//! # fn f(ctx: Ctx) {
//! // an order's id is not a customer's
//! ctx.query::<Order>().join_on::<Customer>(Order::id.eq_col(Customer::id));
//! # }
//! ```
//!
//! ```compile_fail
//! # use celobus_sdk::prelude::*;
//! # #[derive(Entity, Clone, Debug)] pub struct Customer { pub id: Id<Customer>, pub name: String }
//! # #[derive(Entity, Clone, Debug)] pub struct Order { pub id: Id<Order>, pub customer_id: Option<Id<Customer>> }
//! # async fn f(ctx: Ctx) -> Result<()> {
//! // a left-joined entity's columns are optional
//! let rows: Vec<(Id<Order>, String)> = ctx.query::<Order>()
//!     .left_join::<Customer>(Order::customer_id)
//!     .select((Order::id, Customer::name))
//!     .all()
//!     .await?;
//! # Ok(()) }
//! ```

mod select;
mod terms;

use std::marker::PhantomData;
use std::panic::Location;

use celobus_proto as pb;
use pb::{db_condition::Op, query_read::Mode, query_source::Join, syscall, syscall_result};
use serde_json::Value;

use crate::ctx::Ctx;
use crate::db::{Db, Filter, Find, Row};
use crate::error::{Error, Result};
use crate::types::{Id, Timestamp};

mod view;

pub use select::{Cons, Free, Has, Here, Item, Nil, One, Selection, There};
pub use terms::{
    ACol, Agg, Alias, Averaged, Bucket, Bucketable, ColCmp, Column, ColumnCompare, Columns,
    Comparable, Cond, Day, DaysAgo, JoinRef, Label, LabelOf, Nullable, Nullity, Operand, Opt,
    ParseAs, Parsed, QOrder, QueryRow, Queryable, RefValue, Req, alias, avg, canonical, count,
    count_distinct, days_ago, default_alias, exists, exists_as, label, max, min, not_exists, sum,
};
pub use view::{View, ViewValue};

/// A query being built over the root `R` and the sources `S` (at the type level: which entities it has, which of them
/// joined left). Joins, `filter`, `group_by`, `having`, `order_by` and `limit` build it; `select` names the columns of
/// its rows; `all`, `first`, `count` and `page` read it.
pub struct Query<R, S = Cons<R, Req, Nil>> {
    ctx: Ctx,
    engine: pb::Engine,
    tree: pb::Query,
    /// the first mistake made building it (reported by the read)
    error: Option<String>,
    source: &'static Location<'static>,
    _p: PhantomData<fn() -> (R, S)>,
}

/// A page of a query's rows and the token of the next page (none: the last); from the analytic copy, when its data
/// is from.
#[derive(Debug)]
pub struct QueryPage<T> {
    pub rows: Vec<T>,
    pub next: Option<String>,
    pub as_of: Option<Timestamp>,
    /// the generation of the analytic data it was read from (0 when the answer does not say, and from `ctx.query()`)
    pub generation: u64,
}

/// An answer and, from the analytic copy (`ctx.analytics()`), when its data is from (`None` from `ctx.query()`) and
/// the generation of the data it was read from (0 when the answer does not say).
#[derive(Debug)]
pub struct AsOf<T> {
    pub rows: T,
    pub as_of: Option<Timestamp>,
    pub generation: u64,
}

/// The analytic copy of the workspace's entities (`ctx.analytics()`).
#[derive(Clone)]
pub struct Analytics {
    ctx: Ctx,
}

impl Analytics {
    /// `ctx.query::<E>()` on the analytic copy: the same builder, rules and answers, plus `as_of`.
    #[track_caller]
    pub fn query<E: Queryable>(&self) -> Query<E> {
        let mut q = self.ctx.query_any::<E>();
        q.engine = pb::Engine::Analytic;
        q
    }

    /// A snapshot of the analytic copy to ask for (`.load::<E>()…request()`).
    pub fn snapshot(&self) -> crate::lake::Snapshot {
        crate::lake::Snapshot::new(self.ctx.clone())
    }
}

/// When an answer's data is from (unix milliseconds; 0: not an analytic answer).
fn as_of(ms: i64) -> Option<Timestamp> {
    (ms != 0)
        .then(|| chrono::DateTime::from_timestamp_millis(ms))
        .flatten()
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(500, "query_invalid", message)
}

impl<R: Queryable, S> Query<R, S> {
    /// The engine the query is read from (`ctx.lake().query()`).
    pub(crate) fn set_engine(&mut self, engine: pb::Engine) {
        self.engine = engine;
    }

    /// The same query read as another root's rows (an entity's history: `ctx.lake().history()`).
    pub(crate) fn rerooted<R2: Queryable>(self) -> Query<R2, S> {
        Query {
            ctx: self.ctx,
            engine: self.engine,
            tree: self.tree,
            error: self.error,
            source: self.source,
            _p: PhantomData,
        }
    }

    /// The root source of the tree (its history flags).
    pub(crate) fn root_mut(&mut self) -> Option<&mut pb::QuerySource> {
        self.tree.sources.first_mut()
    }

    fn retype<S2>(self) -> Query<R, S2> {
        Query {
            ctx: self.ctx,
            engine: self.engine,
            tree: self.tree,
            error: self.error,
            source: self.source,
            _p: PhantomData,
        }
    }

    fn add_join(
        &mut self,
        entity: String,
        alias: String,
        left: bool,
        on: (String, &'static str, String),
    ) {
        add_join(&mut self.tree, &mut self.error, entity, alias, left, on)
    }

    fn add_join_on(&mut self, entity: String, alias: String, left: bool, on: ColCmp) {
        add_join_on(&mut self.tree, &mut self.error, entity, alias, left, on)
    }

    /// Joins `T` along a ref between it and an entity of the query, either way: `join::<Order>(OrderLine::order_id)`
    /// (the query has `OrderLine`), or `join::<OrderLine>(OrderLine::order_id)` (it has `Order`).
    pub fn join<T: Queryable>(mut self, r: impl JoinRef<T>) -> Query<R, Cons<T, Req, S>> {
        self.add_join(canonical::<T>(), default_alias::<T>(), false, r.join_ref());
        self.retype()
    }

    /// As `join`, keeping the rows that have no `T` (or none the caller may read): `T`'s columns come back as
    /// `Option<V>`.
    pub fn left_join<T: Queryable>(mut self, r: impl JoinRef<T>) -> Query<R, Cons<T, Opt, S>> {
        self.add_join(canonical::<T>(), default_alias::<T>(), true, r.join_ref());
        self.retype()
    }

    /// Joins `T` on two columns of one entity's keys (`Invoice::customer_id.eq_col(Order::customer_id)`): one a
    /// column of `T`, the other of an entity of the query.
    pub fn join_on<T: Queryable>(mut self, on: ColCmp) -> Query<R, Cons<T, Req, S>> {
        self.add_join_on(canonical::<T>(), default_alias::<T>(), false, on);
        self.retype()
    }

    pub fn left_join_on<T: Queryable>(mut self, on: ColCmp) -> Query<R, Cons<T, Opt, S>> {
        self.add_join_on(canonical::<T>(), default_alias::<T>(), true, on);
        self.retype()
    }

    /// Joins an aliased entity (an entity joined twice), inner or left as the alias was made; its columns are the
    /// alias's (`manager.col(..)`).
    pub fn join_as<T: Queryable, N: Nullity>(
        mut self,
        a: &Alias<T, N>,
        r: impl JoinRef<T>,
    ) -> Self {
        self.add_join(canonical::<T>(), a.name.clone(), N::LEFT, r.join_ref());
        self
    }

    pub fn join_on_as<T: Queryable, N: Nullity>(mut self, a: &Alias<T, N>, on: ColCmp) -> Self {
        self.add_join_on(canonical::<T>(), a.name.clone(), N::LEFT, on);
        self
    }

    /// Keeps the rows for which `cond` holds; several `filter`s are all of them.
    pub fn filter(mut self, cond: impl Into<Cond>) -> Self {
        let c = Cond(self.tree.filter.take()) & cond.into();
        self.tree.filter = c.0;
        self
    }

    /// Groups the rows by these columns (a column, or a tuple of them).
    pub fn group_by(mut self, columns: impl Columns) -> Self {
        self.tree.group_by.extend(columns.columns());
        self
    }

    /// Keeps the groups for which `cond` holds (aggregates and grouped columns).
    pub fn having(mut self, cond: impl Into<Cond>) -> Self {
        let c = Cond(self.tree.having.take()) & cond.into();
        self.tree.having = c.0;
        self
    }

    /// Orders the rows (then by their keys); several `order_by`s in turn.
    pub fn order_by(mut self, o: impl Into<QOrder>) -> Self {
        self.tree.order.push(o.into().0);
        self
    }

    /// At most `n` rows (a page: at most 100).
    pub fn limit(mut self, n: u32) -> Self {
        self.tree.limit = n;
        self
    }

    /// The columns of the rows: an item (a column, an aggregate, a `label`) or a tuple of them.
    pub fn select<I, Sel: Selection<S, I>>(mut self, sel: Sel) -> Selected<Sel::Out> {
        self.tree.select = sel.items();
        Selected {
            q: Reader {
                ctx: self.ctx,
                engine: self.engine,
                tree: self.tree,
                error: self.error,
                source: self.source,
            },
            read: Sel::read,
        }
    }

    fn reader(&self) -> Reader {
        Reader {
            ctx: self.ctx.clone(),
            engine: self.engine,
            tree: self.tree.clone(),
            error: self.error.clone(),
            source: self.source,
        }
    }

    /// How many rows (or groups) the query has.
    pub async fn count(&self) -> Result<u64> {
        Ok(self.reader().count().await?.rows)
    }

    /// `count`, with when its data is from.
    pub async fn count_as_of(&self) -> Result<AsOf<u64>> {
        self.reader().count().await
    }
}

fn fail(error: &mut Option<String>, message: String) {
    if error.is_none() {
        *error = Some(message);
    }
}

/// The alias of the one source of `entity` (the default one first).
fn source_of(tree: &pb::Query, entity: &str) -> Option<String> {
    let all: Vec<&pb::QuerySource> = tree.sources.iter().filter(|s| s.entity == entity).collect();
    let default = entity.replace('.', "_");
    if let Some(s) = all.iter().find(|s| s.alias == default) {
        return Some(s.alias.clone());
    }
    match all.as_slice() {
        [one] => Some(one.alias.clone()),
        _ => None,
    }
}

/// Joins `entity` along a ref between it and a source of the tree, either way.
pub(crate) fn add_join(
    tree: &mut pb::Query,
    error: &mut Option<String>,
    entity: String,
    alias: String,
    left: bool,
    on: (String, &'static str, String),
) {
    let (owner, field, target) = on;
    let (l, r) = if target == entity && source_of(tree, &owner).is_some() {
        // forward: the ref is a column of an entity of the query, the new one its target
        let from = source_of(tree, &owner).unwrap_or_default();
        (terms::column(&from, field), terms::column(&alias, "id"))
    } else if owner == entity {
        match source_of(tree, &target) {
            Some(to) => (terms::column(&to, "id"), terms::column(&alias, field)),
            None => {
                fail(
                    error,
                    format!(
                        "join of {entity}: the query has no {target} (or more than one: join with join_on)"
                    ),
                );
                return;
            }
        }
    } else {
        fail(
            error,
            format!(
                "join of {entity}: the query has no {owner} (or more than one: join with join_on)"
            ),
        );
        return;
    };
    push_join(tree, entity, alias, left, l, r);
}

fn push_join(
    tree: &mut pb::Query,
    entity: String,
    alias: String,
    left: bool,
    l: pb::QueryColumn,
    r: pb::QueryColumn,
) {
    tree.sources.push(pb::QuerySource {
        entity,
        alias,
        join: if left { Join::Left } else { Join::Inner } as i32,
        left: Some(l),
        right: Some(r),
        ..Default::default()
    });
}

/// Joins `entity` on two equal columns, one of it and one of a source of the tree.
pub(crate) fn add_join_on(
    tree: &mut pb::Query,
    error: &mut Option<String>,
    entity: String,
    alias: String,
    left: bool,
    on: ColCmp,
) {
    let Some(right) = on.right_column().cloned().filter(|_| on.op == Op::Eq) else {
        fail(
            error,
            format!("join_on of {entity}: two columns are compared with eq_col"),
        );
        return;
    };
    let (l, r) = if right.source == alias {
        (on.left, right)
    } else if on.left.source == alias {
        (right, on.left)
    } else {
        fail(
            error,
            format!("join_on of {entity}: neither column is one of {alias}"),
        );
        return;
    };
    push_join(tree, entity, alias, left, l, r);
}

impl<R: QueryRow, S> Query<R, S> {
    /// The root's rows (at most 1,000: `query_rows_limit` past them): an entity's `Row<E>`, a view's own type.
    pub async fn all(&self) -> Result<Vec<R::Row>> {
        Ok(self.all_as_of().await?.rows)
    }

    /// `all`, with when its data is from.
    pub async fn all_as_of(&self) -> Result<AsOf<Vec<R::Row>>> {
        let r = self.reader().rows(Mode::All, None).await?;
        Ok(AsOf {
            rows: r.rows.iter().map(R::row).collect::<Result<_>>()?,
            as_of: r.as_of,
            generation: r.generation,
        })
    }

    /// The root's first row.
    pub async fn first(&self) -> Result<Option<R::Row>> {
        let r = self.reader().rows(Mode::First, None).await?;
        r.rows.first().map(R::row).transpose()
    }

    /// A page of the root's rows (`limit`, at most 100) after the token of the previous page.
    pub async fn page(&self, token: Option<&str>) -> Result<QueryPage<R::Row>> {
        let r = self.reader().rows(Mode::Page, token).await?;
        Ok(QueryPage {
            rows: r.rows.iter().map(R::row).collect::<Result<_>>()?,
            next: r.next,
            as_of: r.as_of,
            generation: r.generation,
        })
    }
}

/// The repository's reads of a dependency's exported entity, as milestone 3 had them (`ctx.query::<Dep>().get(id)`).
/// On `ctx.analytics().query::<Dep>()` they are refused (500 `internal`) before anything is sent: an analytic query is
/// never answered from the workspace's data.
impl<E: crate::sql::Exported> Query<E, Cons<E, Req, Nil>> {
    /// The context the repository's reads go through (one that refuses them, on an analytic query).
    fn repo(&self) -> Ctx {
        match self.engine {
            pb::Engine::Store => self.ctx.clone(),
            _ => self.ctx.analytic(),
        }
    }

    /// A row by id; 404 `not_found` when there is none, or the caller may not read it.
    pub async fn get(&self, id: &Id<E>) -> Result<Row<E>> {
        Db::<E>::new(self.repo()).get(id).await
    }

    /// A row by id, a soft-deleted one too.
    pub async fn get_with_deleted(&self, id: &Id<E>) -> Result<Row<E>> {
        Db::<E>::new(self.repo()).get_with_deleted(id).await
    }

    /// The rows matching `filter`, as `ctx.db().find`: order, limit and page it, then `all()`, `first()` or `page()`.
    pub fn find(&self, filter: Filter<E>) -> Find<E> {
        Db::<E>::new(self.repo()).find(filter)
    }

    pub async fn count_where(&self, filter: Filter<E>) -> Result<u64> {
        Db::<E>::new(self.repo()).count_where(filter).await
    }

    pub async fn exists(&self, filter: Filter<E>) -> Result<bool> {
        Db::<E>::new(self.repo()).exists(filter).await
    }

    /// Groups of the rows matching `filter`, as `ctx.db().aggregate`, under the provider's rules.
    pub fn aggregate(&self, filter: Filter<E>) -> crate::db::Aggregate<E> {
        Db::<E>::new(self.repo()).aggregate(filter)
    }
}

/// A query whose rows are `T` (`select`): `order_by` and `limit` still apply, then the reads.
pub struct Selected<T> {
    q: Reader,
    read: fn(&[Value]) -> Result<T>,
}

impl<T> Selected<T> {
    /// Orders the rows (then by their keys); several `order_by`s in turn.
    pub fn order_by(mut self, o: impl Into<QOrder>) -> Self {
        self.q.tree.order.push(o.into().0);
        self
    }

    /// At most `n` rows (a page: at most 100).
    pub fn limit(mut self, n: u32) -> Self {
        self.q.tree.limit = n;
        self
    }

    fn decode(&self, rows: Vec<Value>) -> Result<Vec<T>> {
        rows.iter()
            .map(|v| match v {
                Value::Array(items) => (self.read)(items),
                _ => Err(Error::internal(
                    "the platform sent a query row that is not a list",
                )),
            })
            .collect()
    }

    /// Every row (at most 1,000, or `limit`: `query_rows_limit` past them).
    pub async fn all(&self) -> Result<Vec<T>> {
        Ok(self.all_as_of().await?.rows)
    }

    /// `all`, with when its data is from.
    pub async fn all_as_of(&self) -> Result<AsOf<Vec<T>>> {
        let r = self.q.rows(Mode::All, None).await?;
        Ok(AsOf {
            rows: self.decode(r.rows)?,
            as_of: r.as_of,
            generation: r.generation,
        })
    }

    pub async fn first(&self) -> Result<Option<T>> {
        let r = self.q.rows(Mode::First, None).await?;
        Ok(self.decode(r.rows)?.into_iter().next())
    }

    /// A page (`limit`, at most 100) after the token of the previous page: keyset over the order and the keys.
    pub async fn page(&self, token: Option<&str>) -> Result<QueryPage<T>> {
        let r = self.q.rows(Mode::Page, token).await?;
        Ok(QueryPage {
            rows: self.decode(r.rows)?,
            next: r.next,
            as_of: r.as_of,
            generation: r.generation,
        })
    }

    /// How many rows (or groups).
    pub async fn count(&self) -> Result<u64> {
        Ok(self.q.count().await?.rows)
    }

    /// `count`, with when its data is from.
    pub async fn count_as_of(&self) -> Result<AsOf<u64>> {
        self.q.count().await
    }

    /// The query tree the SDK sends (`celobus.v1.Query`).
    pub fn tree(&self) -> &pb::Query {
        &self.q.tree
    }
}

/// What a read sends.
struct Reader {
    ctx: Ctx,
    engine: pb::Engine,
    tree: pb::Query,
    error: Option<String>,
    source: &'static Location<'static>,
}

impl Reader {
    async fn send(
        &self,
        mode: Mode,
        token: Option<&str>,
    ) -> Result<Option<syscall_result::Result>> {
        if let Some(e) = &self.error {
            return Err(invalid(e.clone()));
        }
        self.ctx
            .syscall(syscall::Call::Query(pb::QueryRead {
                request_id: self.ctx.request().id(),
                tx: self.ctx.tx(),
                query: Some(self.tree.clone()),
                mode: mode as i32,
                page_token: token.unwrap_or_default().into(),
                as_system: self.ctx.system(),
                source: format!("{}:{}", self.source.file(), self.source.line()),
                engine: self.engine as i32,
            }))
            .await
    }

    async fn rows(&self, mode: Mode, token: Option<&str>) -> Result<Rows> {
        let Some(syscall_result::Result::DbRows(r)) = self.send(mode, token).await? else {
            return Err(Error::internal(
                "the platform answered a query with something else",
            ));
        };
        let rows = r
            .rows
            .iter()
            .map(|b| {
                serde_json::from_slice(b).map_err(|e| {
                    Error::internal(format!("the platform sent a row that is not JSON: {e}"))
                })
            })
            .collect::<Result<Vec<Value>>>()?;
        let next = (!r.next_page_token.is_empty()).then_some(r.next_page_token);
        Ok(Rows {
            rows,
            next,
            as_of: as_of(r.as_of),
            generation: r.generation,
        })
    }

    async fn count(&self) -> Result<AsOf<u64>> {
        match self.send(Mode::Count, None).await? {
            Some(syscall_result::Result::DbCount(c)) => Ok(AsOf {
                rows: c.count,
                as_of: as_of(c.as_of),
                generation: c.generation,
            }),
            _ => Err(Error::internal(
                "the platform answered a count with something else",
            )),
        }
    }
}

/// The rows of an answer, the next page's token, when the data is from.
struct Rows {
    rows: Vec<Value>,
    next: Option<String>,
    as_of: Option<Timestamp>,
    generation: u64,
}

impl Ctx {
    /// The analytic copy of the workspace's entities: `ctx.analytics().query::<E>()` is `ctx.query::<E>()` read there
    /// (see the module's documentation).
    pub fn analytics(&self) -> Analytics {
        Analytics { ctx: self.clone() }
    }

    /// A typed read across entities, from the root `E`: the plugin's own entity, a dependency's exported one
    /// (`deps::crm::Customer`, which also has the repository's reads: `get`, `find`, `count_where`, `exists`,
    /// `aggregate`), or the directory's (`platform::User`). Inside a transaction (`tx.query::<E>()`) it sees the
    /// transaction's writes; in a system context, the plugin's own entities within its `authz:bypass`, the directory
    /// as a member reads it, and a dependency's entity under the provider's rules for the caller (never more than the
    /// caller reads without the system context).
    #[track_caller]
    pub fn query<E: Queryable>(&self) -> Query<E> {
        const {
            assert!(
                !E::OLAP,
                "an entity stored in OLAP is read with ctx.analytics().query(), never ctx.query()"
            )
        };
        self.query_any::<E>()
    }

    /// [`Ctx::query`] of any source, an entity stored in OLAP too (`ctx.analytics()`, which reads it).
    #[track_caller]
    pub(crate) fn query_any<E: Queryable>(&self) -> Query<E> {
        Query {
            ctx: self.clone(),
            engine: pb::Engine::Store,
            tree: pb::Query {
                sources: vec![pb::QuerySource {
                    entity: canonical::<E>(),
                    alias: default_alias::<E>(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            error: None,
            source: Location::caller(),
            _p: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests;
