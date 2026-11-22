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
use crate::message::Message;
use crate::types::Id;

pub use select::{Cons, Free, Has, Here, Item, Nil, One, Selection, There};
pub use terms::{
    ACol, Agg, Alias, Averaged, ColCmp, Column, ColumnCompare, Columns, Comparable, Cond, JoinRef,
    Label, LabelOf, Nullable, Nullity, Opt, QOrder, Queryable, RefValue, Req, alias, avg,
    canonical, count, count_distinct, default_alias, exists, exists_as, label, max, min,
    not_exists, sum,
};

/// A query being built over the root `R` and the sources `S` (at the type level: which entities it has, which of them
/// joined left). Joins, `filter`, `group_by`, `having`, `order_by` and `limit` build it; `select` names the columns of
/// its rows; `all`, `first`, `count` and `page` read it.
pub struct Query<R, S = Cons<R, Req, Nil>> {
    ctx: Ctx,
    tree: pb::Query,
    /// the first mistake made building it (reported by the read)
    error: Option<String>,
    source: &'static Location<'static>,
    _p: PhantomData<fn() -> (R, S)>,
}

/// A page of a query's rows and the token of the next page (none: the last).
#[derive(Debug)]
pub struct QueryPage<T> {
    pub rows: Vec<T>,
    pub next: Option<String>,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(500, "query_invalid", message)
}

impl<R: Queryable, S> Query<R, S> {
    fn retype<S2>(self) -> Query<R, S2> {
        Query {
            ctx: self.ctx,
            tree: self.tree,
            error: self.error,
            source: self.source,
            _p: PhantomData,
        }
    }

    fn fail(&mut self, message: String) {
        if self.error.is_none() {
            self.error = Some(message);
        }
    }

    /// The alias of the one source of `entity` (the default one first).
    fn source_of(&self, entity: &str) -> Option<String> {
        let all: Vec<&pb::QuerySource> = self
            .tree
            .sources
            .iter()
            .filter(|s| s.entity == entity)
            .collect();
        let default = entity.replace('.', "_");
        if let Some(s) = all.iter().find(|s| s.alias == default) {
            return Some(s.alias.clone());
        }
        match all.as_slice() {
            [one] => Some(one.alias.clone()),
            _ => None,
        }
    }

    fn add_join(
        &mut self,
        entity: String,
        alias: String,
        left: bool,
        on: (String, &'static str, String),
    ) {
        let (owner, field, target) = on;
        let (l, r) = if target == entity && self.source_of(&owner).is_some() {
            // forward: the ref is a column of an entity of the query, the new one its target
            let from = self.source_of(&owner).unwrap_or_default();
            (terms::column(&from, field), terms::column(&alias, "id"))
        } else if owner == entity {
            match self.source_of(&target) {
                Some(to) => (terms::column(&to, "id"), terms::column(&alias, field)),
                None => {
                    self.fail(format!(
                        "join of {entity}: the query has no {target} (or more than one: join with join_on)"
                    ));
                    return;
                }
            }
        } else {
            self.fail(format!(
                "join of {entity}: the query has no {owner} (or more than one: join with join_on)"
            ));
            return;
        };
        self.push_join(entity, alias, left, l, r);
    }

    fn push_join(
        &mut self,
        entity: String,
        alias: String,
        left: bool,
        l: pb::QueryColumn,
        r: pb::QueryColumn,
    ) {
        self.tree.sources.push(pb::QuerySource {
            entity,
            alias,
            join: if left { Join::Left } else { Join::Inner } as i32,
            left: Some(l),
            right: Some(r),
        });
    }

    fn add_join_on(&mut self, entity: String, alias: String, left: bool, on: ColCmp) {
        if on.op != Op::Eq {
            self.fail(format!(
                "join_on of {entity}: the columns are compared with eq_col"
            ));
            return;
        }
        let (l, r) = if on.right.source == alias {
            (on.left, on.right)
        } else if on.left.source == alias {
            (on.right, on.left)
        } else {
            self.fail(format!(
                "join_on of {entity}: neither column is one of {alias}"
            ));
            return;
        };
        self.push_join(entity, alias, left, l, r);
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
            tree: self.tree.clone(),
            error: self.error.clone(),
            source: self.source,
        }
    }

    /// How many rows (or groups) the query has.
    pub async fn count(&self) -> Result<u64> {
        self.reader().count().await
    }
}

impl<R: Queryable + Message, S> Query<R, S> {
    /// The root's rows (at most 1,000: `query_rows_limit` past them).
    pub async fn all(&self) -> Result<Vec<Row<R>>> {
        let (rows, _) = self.reader().rows(Mode::All, None).await?;
        rows.iter().map(|v| Row::<R>::from_json(v, "")).collect()
    }

    /// The root's first row.
    pub async fn first(&self) -> Result<Option<Row<R>>> {
        let (rows, _) = self.reader().rows(Mode::First, None).await?;
        rows.first().map(|v| Row::<R>::from_json(v, "")).transpose()
    }

    /// A page of the root's rows (`limit`, at most 100) after the token of the previous page.
    pub async fn page(&self, token: Option<&str>) -> Result<QueryPage<Row<R>>> {
        let (rows, next) = self.reader().rows(Mode::Page, token).await?;
        Ok(QueryPage {
            rows: rows
                .iter()
                .map(|v| Row::<R>::from_json(v, ""))
                .collect::<Result<_>>()?,
            next,
        })
    }
}

/// The repository's reads of a dependency's exported entity, as milestone 3 had them (`ctx.query::<Dep>().get(id)`).
impl<E: crate::sql::Exported> Query<E, Cons<E, Req, Nil>> {
    /// A row by id; 404 `not_found` when there is none, or the caller may not read it.
    pub async fn get(&self, id: &Id<E>) -> Result<Row<E>> {
        Db::<E>::new(self.ctx.clone()).get(id).await
    }

    /// A row by id, a soft-deleted one too.
    pub async fn get_with_deleted(&self, id: &Id<E>) -> Result<Row<E>> {
        Db::<E>::new(self.ctx.clone()).get_with_deleted(id).await
    }

    /// The rows matching `filter`, as `ctx.db().find`: order, limit and page it, then `all()`, `first()` or `page()`.
    pub fn find(&self, filter: Filter<E>) -> Find<E> {
        Db::<E>::new(self.ctx.clone()).find(filter)
    }

    pub async fn count_where(&self, filter: Filter<E>) -> Result<u64> {
        Db::<E>::new(self.ctx.clone()).count_where(filter).await
    }

    pub async fn exists(&self, filter: Filter<E>) -> Result<bool> {
        Db::<E>::new(self.ctx.clone()).exists(filter).await
    }

    /// Groups of the rows matching `filter`, as `ctx.db().aggregate`, under the provider's rules.
    pub fn aggregate(&self, filter: Filter<E>) -> crate::db::Aggregate<E> {
        Db::<E>::new(self.ctx.clone()).aggregate(filter)
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
        let (rows, _) = self.q.rows(Mode::All, None).await?;
        self.decode(rows)
    }

    pub async fn first(&self) -> Result<Option<T>> {
        let (rows, _) = self.q.rows(Mode::First, None).await?;
        Ok(self.decode(rows)?.into_iter().next())
    }

    /// A page (`limit`, at most 100) after the token of the previous page: keyset over the order and the keys.
    pub async fn page(&self, token: Option<&str>) -> Result<QueryPage<T>> {
        let (rows, next) = self.q.rows(Mode::Page, token).await?;
        Ok(QueryPage {
            rows: self.decode(rows)?,
            next,
        })
    }

    /// How many rows (or groups).
    pub async fn count(&self) -> Result<u64> {
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
            }))
            .await
    }

    async fn rows(&self, mode: Mode, token: Option<&str>) -> Result<(Vec<Value>, Option<String>)> {
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
        Ok((rows, next))
    }

    async fn count(&self) -> Result<u64> {
        match self.send(Mode::Count, None).await? {
            Some(syscall_result::Result::DbCount(c)) => Ok(c.count),
            _ => Err(Error::internal(
                "the platform answered a count with something else",
            )),
        }
    }
}

impl Ctx {
    /// A typed read across entities, from the root `E`: the plugin's own entity, a dependency's exported one
    /// (`deps::crm::Customer`, which also has the repository's reads: `get`, `find`, `count_where`, `exists`,
    /// `aggregate`), or the directory's (`platform::User`). Inside a transaction (`tx.query::<E>()`) it sees the
    /// transaction's writes; in a system context, the plugin's own entities within its `authz:bypass`, the directory
    /// as a member reads it, and a dependency's entity under the provider's rules for the caller (never more than the
    /// caller reads without the system context).
    #[track_caller]
    pub fn query<E: Queryable>(&self) -> Query<E> {
        Query {
            ctx: self.clone(),
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
