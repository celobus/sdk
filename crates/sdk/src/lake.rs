//! The workspace's Data Lake: tables of rows kept for analysis and for BI tools, grouped in layers the plugin names
//! (`raw`, `core`, `mart`: the platform names none). A plugin declares its layers and its lake tables, writes them
//! from its code (usually a job: `replace`, `append`, `merge`) and reads them with the same query builder as its
//! entities.
//!
//! ```ignore
//! pub const RAW: Layer = Layer::new("raw").title("As it arrived");
//!
//! /// Stock as the ERP sends it.
//! #[derive(LakeTable, Clone, Debug)]
//! #[lake(layer = RAW, name = "stock_file", partition(loaded_on))]
//! pub struct StockFileRow { pub loaded_on: Date, pub sku: String, pub warehouse: String, pub on_hand: String }
//!
//! // Plugin::new(..).role(ANALYST).layer(RAW).lake_table::<StockFileRow>().lake_read::<StockFileRow>([ANALYST])
//!
//! async fn run(ctx: Ctx, _: Nightly) -> Result<()> {
//!     let day = today();
//!     let rows = fetch_from_erp(day).await?;
//!     ctx.lake().table::<StockFileRow>().replace(StockFileRow::loaded_on.eq(day), rows).await?;
//!     Ok(())
//! }
//!
//! let low = ctx.lake().query::<StockFileRow>()
//!     .filter(StockFileRow::loaded_on.eq(day))
//!     .order_by(StockFileRow::sku.asc())
//!     .limit(50)
//!     .all()
//!     .await?;                                               // Vec<StockFileRow>
//! ```
//!
//! **Writes.** A plugin writes only its own tables, at most 1,000,000 rows a write (`lake_rows_limit`; larger loads go
//! partition by partition) in at most 1,000 partitions (`lake_partitions_limit`: write fewer days at a time). Each write is one commit: its rows are all there, or none. The writes of one table run one
//! at a time across the workspace (`lake_busy` after 60 s of waiting).
//!
//! * `replace(filter, rows)` swaps the partitions the filter names for `rows`: the filter compares partition columns
//!   only, and a row it does not select refuses the write (`invalid`). A table without partition columns is replaced
//!   whole (`Filter::all()`). Run again, it leaves the same table.
//! * `append(rows)` adds the rows once per write key: a write whose key the table has had within 32 days (longer
//!   than a failed run can be retried) adds nothing ([`Written::skipped`]). In a job's run the key is, by default, the run's id and the call's number, so an
//!   attempt that runs again appends nothing twice; elsewhere `.key(..)` gives it, and an append without one is
//!   refused (`invalid`): a write may answer an error although it was written, and only its key makes running it again
//!   safe.
//! * `merge(rows)` writes each row over the row with the same declared key (`key(..)`), or adds it.
//!
//! **Reads.** `ctx.lake().query::<T>()` takes the joins, filters, groups, aggregates, time buckets, orders and pages of
//! `ctx.query()`, over lake tables only (another plugin's table it declares too: `Plugin::lake_dependency`). Whoever
//! holds one of a table's read roles (`Plugin::lake_read`) reads every row of it; there are no row rules. The plugin's
//! system context (`ctx.as_system(..)`) reads only its own tables declared so (`Plugin::lake_read_as_system`). A query
//! runs at most 10 s (`lake_time_limit`), within the lake's memory (`lake_memory_limit`); four of the workspace at once
//! (`lake_busy`). Decimals are numbers with at most 20 digits before the point and 18 after (a longer one is refused at
//! write) and read back without trailing zeros. `unavailable` when the workspace has no lake.
//!
//! **Models.** A lake table may be computed by the platform from other tables, when the plugin asks: its [`Model`]
//! (`Plugin::model`) is a view's query whose row is the table's, over lake tables or over the plugin's own entities
//! and analytic views (never both). `ctx.lake().refresh::<T>()` runs it and replaces the whole table with its rows,
//! `.partition(filter)` only the partitions the filter names, in one commit; the rows never pass through the plugin.
//! A refresh holds at most 1,000,000 rows (`lake_model_rows_limit`: refresh by partition); a text it parses that does
//! not read as its type fails it, naming the column (`lake_model_parse`); after the model's declaration changed, a
//! refresh of partitions answers how many partitions still hold the earlier declaration's rows
//! ([`Refreshed::stale_partitions`]): the table mixes them until it is 0. A model over entities or views reads
//! them as the plugin's system context (its bypass of `read` must cover every entity under it, `capability_missing`
//! otherwise), from the workspace's analytic copy (`unavailable` while it is loading or its data is older than the
//! workspace's limit; [`Refreshed::as_of`] says when
//! its data is from), and the read is audited.
//!
//! ```ignore
//! fn stock() -> Model<Stock> {
//!     Model::<Stock>::from::<StockFileRow>()
//!         .filter(!StockFileRow::sku.eq(String::new()))
//!         .col(Stock::day, StockFileRow::loaded_on)
//!         .col(Stock::sku, StockFileRow::sku)
//!         .col(Stock::on_hand, StockFileRow::on_hand.parse::<Decimal>())
//! }
//! // Plugin::new(..).lake_table::<StockFileRow>().lake_table::<Stock>().model(stock())
//! ctx.lake().refresh::<Stock>().partition(Stock::day.eq(day)).await?;
//! ```
//!
//! **Changes.** A later version may add a layer, a table, an optional field; a table keeps its layer, its partition and
//! key columns and the types of its fields (`celobus plugin check` refuses the rest, whatever the version). A table no
//! longer declared stays in the lake. Rows written before a field was added read it as absent.
//!
//! A lake table's row is no entity: the repository (`ctx.db()`) does not take it.
//!
//! ```compile_fail
//! # use celobus_sdk::prelude::*;
//! const RAW: Layer = Layer::new("raw");
//! #[derive(LakeTable, Clone, Debug)]
//! #[lake(layer = RAW)]
//! pub struct Stock { pub sku: String }
//! # fn f(ctx: Ctx) {
//! ctx.db::<Stock>();
//! # }
//! ```

use std::{
    future::{Future, IntoFuture},
    marker::PhantomData,
    pin::Pin,
};

use bytes::Bytes;
use celobus_proto as pb;
use pb::{lake_write::Mode, syscall, syscall_result};

use crate::{
    ctx::Ctx,
    db::Filter,
    error::{Error, Result},
    message::Message,
    query::{
        Alias, ColCmp, Columns, Comparable, Cond, JoinRef, Nullity, Query, Queryable, View,
        ViewValue,
    },
    types::Timestamp,
    ui::Field,
};

pub use celobus_descriptor::lake::{CHUNK_BYTES, CHUNK_ROWS, WRITE_ROWS};

pub mod history;
pub use history::{History, HistoryRow};

/// A layer of the lake: a name (snake_case, at most 32 characters) and a title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layer {
    name: &'static str,
    title: &'static str,
}

impl Layer {
    pub const fn new(name: &'static str) -> Self {
        Self { name, title: "" }
    }

    pub const fn title(self, title: &'static str) -> Self {
        Self { title, ..self }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn get_title(&self) -> &'static str {
        self.title
    }
}

/// A lake table's row (`#[derive(LakeTable)]`): its layer, its name, its partition and key columns; for another
/// plugin's table (`#[lake(plugin = "retail")]`), its owner.
pub trait LakeTable: Queryable + Message {
    const LAYER: Layer;
    /// The table's name in the plugin (`stock_file`).
    const TABLE: &'static str;
    const PARTITION: &'static [&'static str];
    const KEY: &'static [&'static str];
}

/// The lake (`ctx.lake()`).
#[derive(Clone)]
pub struct Lake {
    ctx: Ctx,
}

impl Ctx {
    /// The workspace's Data Lake: write the plugin's lake tables, query them (see [`crate::lake`]).
    pub fn lake(&self) -> Lake {
        Lake { ctx: self.clone() }
    }
}

impl Lake {
    /// The writes of one of the plugin's lake tables.
    pub fn table<T: LakeTable>(&self) -> LakeWriter<T> {
        LakeWriter {
            ctx: self.ctx.clone(),
            _t: PhantomData,
        }
    }

    /// Runs the model of the plugin's lake table `T` (`Plugin::model`): the platform computes its rows and replaces
    /// the whole table with them (or, with `.partition(filter)`, the partitions the filter names), in one commit.
    #[track_caller]
    pub fn refresh<T: LakeTable>(&self) -> Refresh<T> {
        Refresh {
            ctx: self.ctx.clone(),
            filter: None,
            _t: PhantomData,
        }
    }

    /// A query of lake tables from the root `T` (the plugin's own, or another plugin's it declares it reads): the
    /// builder of `ctx.query()`.
    #[track_caller]
    pub fn query<T: LakeTable>(&self) -> Query<T> {
        let mut q = self.ctx.query::<T>();
        q.set_engine(pb::Engine::Lake);
        q
    }

    /// A query of the history of `E` (see [`history`]): its changes, filtered, grouped and ordered by `E`'s kept
    /// columns and the history's own ([`history::valid_from`], ...); `.each_day()` reads it per day. Its rows without
    /// `select` are [`HistoryRow<E>`].
    #[track_caller]
    pub fn history<E: crate::message::Entity + Queryable>(
        &self,
    ) -> Query<History<E>, crate::query::Cons<E, crate::query::Req, crate::query::Nil>> {
        let mut q = self.ctx.query::<E>();
        q.set_engine(pb::Engine::Lake);
        if let Some(root) = q.root_mut() {
            root.history = true;
        }
        q.rerooted::<History<E>>()
    }
}

impl<E: crate::message::Entity + Queryable, S> Query<History<E>, S> {
    /// One row per day each version was in force at the day's last moment, in the workspace's time zone, with its day
    /// ([`history::day`]).
    pub fn each_day(mut self) -> Self {
        if let Some(root) = self.root_mut() {
            root.each_day = true;
        }
        self
    }
}

/// What a write did: the rows it carried, and whether an append was skipped (its key was written before).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Written {
    pub rows: u64,
    pub skipped: bool,
}

/// The writes of the lake table `T`.
pub struct LakeWriter<T> {
    ctx: Ctx,
    _t: PhantomData<fn() -> T>,
}

impl<T: LakeTable> LakeWriter<T> {
    /// Swaps the partitions `filter` names (partition columns only) for `rows`; a table without partition columns is
    /// replaced whole with `Filter::all()`.
    pub async fn replace(
        &self,
        filter: Filter<T>,
        rows: impl IntoIterator<Item = T>,
    ) -> Result<Written> {
        send::<T>(
            &self.ctx,
            Mode::Replace,
            filter.node().cloned(),
            String::new(),
            rows,
        )
        .await
    }

    /// Writes each row over the row with the same key, or adds it.
    pub async fn merge(&self, rows: impl IntoIterator<Item = T>) -> Result<Written> {
        send::<T>(&self.ctx, Mode::Merge, None, String::new(), rows).await
    }

    /// Adds the rows, once per write key (`.key(..)`; in a job's run, by default, the run's id and the call's number).
    #[track_caller]
    pub fn append<I: IntoIterator<Item = T>>(&self, rows: I) -> Append<T> {
        let key = {
            let job = self.ctx.job();
            if job.is_run() {
                format!("{}#{}", job.run_id(), self.ctx.next_append())
            } else {
                String::new()
            }
        };
        Append {
            ctx: self.ctx.clone(),
            rows: rows.into_iter().collect(),
            key,
        }
    }
}

/// An append, sent when awaited.
pub struct Append<T> {
    ctx: Ctx,
    rows: Vec<T>,
    key: String,
}

impl<T: LakeTable> Append<T> {
    /// The write key: a write whose key the table has adds nothing (at most 512 bytes).
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }
}

impl<T: LakeTable + Send + 'static> IntoFuture for Append<T> {
    type Output = Result<Written>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<Written>> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            if self.key.trim().is_empty() {
                return Err(Error::new(
                    400,
                    "invalid",
                    format!(
                        "an append of {} outside a job's run needs .key(..): a write run again after an error appends \
                         once only under its key",
                        T::TABLE
                    ),
                ));
            }
            send::<T>(&self.ctx, Mode::Append, None, self.key, self.rows).await
        })
    }
}

/// The rows as chunks of at most [`CHUNK_ROWS`] rows and [`CHUNK_BYTES`] bytes.
pub(crate) fn chunks<T: Message>(rows: impl IntoIterator<Item = T>) -> Result<Vec<Vec<Bytes>>> {
    let mut out: Vec<Vec<Bytes>> = vec![vec![]];
    let (mut size, mut n) = (0usize, 0u64);
    for r in rows {
        let b = Bytes::from(serde_json::to_vec(&r.to_json())?);
        if b.len() > CHUNK_BYTES {
            return Err(Error::new(
                422,
                "bulk_limit",
                format!("a row of {} is over {} MiB", T::NAME, CHUNK_BYTES >> 20),
            ));
        }
        n += 1;
        if n > WRITE_ROWS {
            return Err(Error::new(
                422,
                "lake_rows_limit",
                format!("a write holds at most {WRITE_ROWS} rows: write the rest in another write"),
            ));
        }
        let last = out.last_mut().expect("a chunk");
        if last.len() == CHUNK_ROWS || size + b.len() > CHUNK_BYTES {
            out.push(vec![]);
            size = 0;
        }
        size += b.len();
        out.last_mut().expect("a chunk").push(b);
    }
    Ok(out)
}

async fn send<T: LakeTable>(
    ctx: &Ctx,
    mode: Mode,
    filter: Option<pb::DbFilter>,
    key: String,
    rows: impl IntoIterator<Item = T>,
) -> Result<Written> {
    if T::OWNER.is_some() {
        return Err(Error::internal(format!(
            "{} is another plugin's lake table: a plugin writes only its own",
            T::TABLE
        )));
    }
    let parts = chunks(rows)?;
    let count = parts.len();
    let mut write = 0;
    for (i, part) in parts.into_iter().enumerate() {
        let first = write == 0;
        let w = pb::LakeWrite {
            request_id: ctx.request().id(),
            table: T::TABLE.into(),
            mode: mode as i32,
            filter: if first { filter.clone() } else { None },
            key: if first { key.clone() } else { String::new() },
            rows: part,
            write,
            last: i + 1 == count,
        };
        match ctx.syscall(syscall::Call::LakeWrite(w)).await? {
            Some(syscall_result::Result::LakeWritten(out)) => {
                write = out.write;
                if out.committed || out.skipped {
                    return Ok(Written {
                        rows: out.rows,
                        skipped: out.skipped,
                    });
                }
            }
            _ => {
                return Err(Error::internal(
                    "the platform answered a lake write with something else",
                ));
            }
        }
    }
    Err(Error::internal("the platform did not end the lake write"))
}

/// What a refresh did: the rows it wrote, and when the data of the analytic copy a model over entities or views read
/// is from (none for a model over lake tables).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refreshed {
    pub rows: u64,
    pub as_of: Option<Timestamp>,
    /// After a refresh of partitions: how many partitions the table still holds that an earlier declaration of the
    /// model computed (0: every row is this declaration's).
    pub stale_partitions: u64,
}

/// A refresh of the model of `T`, run when awaited.
pub struct Refresh<T> {
    ctx: Ctx,
    filter: Option<pb::DbFilter>,
    _t: PhantomData<fn() -> T>,
}

impl<T: LakeTable> Refresh<T> {
    /// Only the partitions `filter` names (partition columns only, as `replace`'s): the model's rows of those
    /// partitions replace them. After the model's declaration changed, the answer says how many partitions still hold
    /// the earlier declaration's rows ([`Refreshed::stale_partitions`]). A filter that names no partition (`Filter::all()`) is sent as
    /// such and refused (`invalid`): a whole refresh is the one without `.partition`.
    pub fn partition(mut self, filter: Filter<T>) -> Self {
        self.filter = Some(filter.node().cloned().unwrap_or_default());
        self
    }
}

impl<T: LakeTable + Send + 'static> IntoFuture for Refresh<T> {
    type Output = Result<Refreshed>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<Refreshed>> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            if T::OWNER.is_some() {
                return Err(Error::internal(format!(
                    "{} is another plugin's lake table: a plugin refreshes only its own",
                    T::TABLE
                )));
            }
            let r = pb::LakeRefresh {
                request_id: self.ctx.request().id(),
                table: T::TABLE.into(),
                filter: self.filter,
            };
            match self.ctx.syscall(syscall::Call::LakeRefresh(r)).await? {
                Some(syscall_result::Result::LakeRefreshed(out)) => Ok(Refreshed {
                    rows: out.rows,
                    as_of: (out.as_of > 0)
                        .then(|| chrono::DateTime::from_timestamp_millis(out.as_of))
                        .flatten(),
                    stale_partitions: out.stale_partitions,
                }),
                _ => Err(Error::internal(
                    "the platform answered a lake refresh with something else",
                )),
            }
        })
    }
}

/// How the platform computes the lake table `T` from other tables (`Plugin::model`): a query whose sources are lake
/// tables (the plugin's own, or another plugin's it reads: `Plugin::lake_dependency`), or the plugin's own entities
/// and analytic views, never both; its columns are `T`'s fields, each given once with `col`, as a view's. The
/// builder is a view's ([`View`]): joins, `filter`, `group_by`, `having`, time buckets, `days_ago`; a text column of a
/// lake table may be parsed (`.parse::<Decimal>()`).
///
/// ```ignore
/// fn stock() -> Model<Stock> {
///     Model::<Stock>::from::<StockFileRow>()
///         .filter(!StockFileRow::sku.eq(""))
///         .col(Stock::day, StockFileRow::loaded_on)
///         .col(Stock::sku, StockFileRow::sku)
///         .col(Stock::on_hand, StockFileRow::on_hand.parse::<Decimal>())
/// }
/// // Plugin::new(..).lake_table::<StockFileRow>().lake_table::<Stock>().model(stock())
/// // a job: ctx.lake().refresh::<Stock>().partition(Stock::day.eq(day)).await?;
/// ```
///
/// A model over entities or views reads them as the plugin's system context: the plugin's bypass of `read` must
/// cover every entity under it (`Plugin::bypass`), it names no encrypted field, and a masked field only when it says
/// [`Model::unmasked`] (whoever reads the lake table reads every row of it). It reads the workspace's analytic copy as
/// it is on the replica that runs the refresh ([`Refreshed::as_of`]). `celobus plugin check` refuses a model over an
/// unknown table or column, of another type than its field, mixing lake tables with entities or views, reading its
/// own table (directly or through other models), or larger than a view may be.
pub struct Model<T> {
    view: View<T>,
    unmasked: bool,
}

impl<T: LakeTable> Model<T> {
    /// A model whose root is `S`: a lake table, or an entity or an analytic view of the plugin.
    pub fn from<S: Queryable>() -> Self {
        Model {
            view: View::<T>::from::<S>(),
            unmasked: false,
        }
    }

    /// A model whose root is an entity's history (`history::history::<Ticket>()`, `.each_day()` for one row per day):
    /// its columns are the entity's kept ones and the history's own (`history::valid_from::<Ticket>()`, ...). Such a
    /// model reads the lake: it may join lake tables, never entities. It reads one of the plugin's own entities'
    /// histories, as the plugin's system context: the history must say so (`Plugin::history_read_as_system`).
    /// Another plugin's history (an export lets callers read it, not a model copy it) does not compile:
    ///
    /// ```compile_fail
    /// use celobus_sdk::prelude::*;
    /// use celobus_sdk::lake::history;
    /// const MART: Layer = Layer::new("mart");
    /// #[derive(Entity, Clone, Debug)]
    /// #[entity(plugin = "helpdesk")]
    /// pub struct Ticket { pub id: Id<Ticket>, pub status: String }
    /// #[derive(LakeTable, Clone, Debug)]
    /// #[lake(layer = MART)]
    /// pub struct Copy { pub status: Option<String> }
    /// fn m() -> Model<Copy> {
    ///     Model::<Copy>::from_history(history::history::<Ticket>()).col(Copy::status, Ticket::status)
    /// }
    /// fn main() { let _ = m(); }
    /// ```
    pub fn from_history<E: crate::message::Entity + Queryable>(source: history::Source<E>) -> Self {
        const {
            assert!(
                E::PLUGIN.is_none(),
                "a model reads the plugin's own entities' histories only, never another plugin's"
            )
        };
        Model {
            view: View::<T>::from::<E>().history_root(source.each_day),
            unmasked: false,
        }
    }

    /// Joins `S` along a ref between it and a source of the model, either way.
    pub fn join<S: Queryable>(self, r: impl JoinRef<S>) -> Self {
        Model {
            view: self.view.join::<S>(r),
            ..self
        }
    }

    /// As `join`, keeping the rows that have no `S`.
    pub fn left_join<S: Queryable>(self, r: impl JoinRef<S>) -> Self {
        Model {
            view: self.view.left_join::<S>(r),
            ..self
        }
    }

    /// Joins `S` on two equal columns (`Stock::sku.eq_col(Product::sku)`).
    pub fn join_on<S: Queryable>(self, on: ColCmp) -> Self {
        Model {
            view: self.view.join_on::<S>(on),
            ..self
        }
    }

    pub fn left_join_on<S: Queryable>(self, on: ColCmp) -> Self {
        Model {
            view: self.view.left_join_on::<S>(on),
            ..self
        }
    }

    /// Joins an aliased source (one joined twice).
    pub fn join_as<S: Queryable, N: Nullity>(self, a: &Alias<S, N>, r: impl JoinRef<S>) -> Self {
        Model {
            view: self.view.join_as::<S, N>(a, r),
            ..self
        }
    }

    /// Keeps the rows for which `cond` holds; several `filter`s are all of them.
    pub fn filter(self, cond: impl Into<Cond>) -> Self {
        Model {
            view: self.view.filter(cond),
            ..self
        }
    }

    /// Groups the rows by these columns: each must be a column of the model.
    pub fn group_by(self, columns: impl Columns) -> Self {
        Model {
            view: self.view.group_by(columns),
            ..self
        }
    }

    /// Keeps the groups for which `cond` holds.
    pub fn having(self, cond: impl Into<Cond>) -> Self {
        Model {
            view: self.view.having(cond),
            ..self
        }
    }

    /// The table's column `field` is `value`: a column of a source (a bucket of it, a parsed text) or, grouped, an
    /// aggregate, of the field's type.
    pub fn col<V, X: ViewValue>(self, field: Field<T, V>, value: X) -> Self
    where
        X::V: Comparable<V>,
    {
        Model {
            view: self.view.col(field, value),
            ..self
        }
    }

    /// The masked fields of the entities under the model go to the lake as they are: the lake has no masks, and
    /// whoever reads `T` reads every row of it.
    pub fn unmasked(mut self) -> Self {
        self.unmasked = true;
        self
    }

    /// The model as the descriptor declares it, or the first mistake made building it.
    pub(crate) fn decl(
        &self,
    ) -> std::result::Result<celobus_descriptor::lake_model::LakeModel, String> {
        let v = self.view.decl(String::new())?;
        Ok(celobus_descriptor::lake_model::LakeModel {
            table: T::TABLE.to_string(),
            sources: v.sources,
            filter: v.filter,
            group_by: v.group_by,
            having: v.having,
            columns: v.columns,
            unmasked: self.unmasked,
        })
    }
}

/// How the platform fills the entity stored in OLAP `E` (`#[entity(storage = olap, key(..))]`, `Plugin::load`) from
/// the lake, when the plugin asks for a snapshot that loads it: a view's query over lake tables whose row is `E`'s
/// fields (every field but `id`, which is made from `key(..)`; an optional field may be left out and reads absent).
/// Its sources are the plugin's own lake tables and entities' histories (read as the plugin's system context: each
/// declared so, `Plugin::lake_read_as_system` and `Plugin::history_read_as_system`) and another plugin's lake table
/// it declares it reads (`Plugin::lake_dependency`) whose owner lets other plugins copy it
/// (`Plugin::lake_export_to_models`); never an entity or an analytic view, which a view of the plugin joins with `E`.
///
/// ```ignore
/// /// One metric of one store per day.
/// #[derive(Entity, Clone, Debug)]
/// #[entity(storage = olap, key(day, store, metric))]
/// pub struct StoreMetric { pub id: Id<StoreMetric>, pub day: Date, pub store: Id<Store>, pub metric: String,
///                          pub value: f64 }
///
/// fn store_metric_load() -> Load<StoreMetric> {
///     Load::<StoreMetric>::from::<StoreMetricsMart>()
///         .filter(StoreMetricsMart::day.ge(days_ago(400)))     // only what reports need, not the whole table
///         .col(StoreMetric::day, StoreMetricsMart::day)
///         .col(StoreMetric::store, StoreMetricsMart::store_id)
///         .col(StoreMetric::metric, StoreMetricsMart::metric)
///         .col(StoreMetric::value, StoreMetricsMart::value)
/// }
/// // Plugin::new(..).entity::<StoreMetric>().access::<StoreMetric>(..).load(store_metric_load())
/// // the nightly job, last: ctx.analytics().snapshot().load::<StoreMetric>().request().await?.wait().await?;
/// ```
///
/// `celobus plugin check` refuses a load over an unknown table or column, of another type than its field, without a
/// required field, giving `id`, reading an entity, a view or another plugin's table it does not declare, or larger
/// than a view may be.
///
/// Such an entity is read with `ctx.analytics().query()` only:
///
/// ```
/// use celobus_sdk::prelude::*;
/// #[derive(Entity, Clone, Debug)]
/// #[entity(storage = olap, key(day))]
/// pub struct Daily { pub id: Id<Daily>, pub day: Date, pub visits: i64 }
/// fn read(ctx: &Ctx) { let _ = ctx.analytics().query::<Daily>(); }
/// fn main() { let _: fn(&Ctx) = read; }
/// ```
///
/// `ctx.db()` and `ctx.query()` of it do not compile:
///
/// ```compile_fail
/// use celobus_sdk::prelude::*;
/// #[derive(Entity, Clone, Debug)]
/// #[entity(storage = olap, key(day))]
/// pub struct Daily { pub id: Id<Daily>, pub day: Date, pub visits: i64 }
/// fn read(ctx: &Ctx) { let _ = ctx.query::<Daily>(); }
/// fn main() { let _: fn(&Ctx) = read; }
/// ```
///
/// ```compile_fail
/// use celobus_sdk::prelude::*;
/// #[derive(Entity, Clone, Debug)]
/// #[entity(storage = olap, key(day))]
/// pub struct Daily { pub id: Id<Daily>, pub day: Date, pub visits: i64 }
/// fn write(ctx: &Ctx) { let _ = ctx.db::<Daily>(); }
/// fn main() { let _: fn(&Ctx) = write; }
/// ```
pub struct Load<E> {
    view: View<E>,
}

impl<E: crate::message::Entity + Queryable + Message> Load<E> {
    /// A load whose root is the lake table `S`.
    pub fn from<S: Queryable>() -> Self {
        Load {
            view: View::<E>::from::<S>(),
        }
    }

    /// A load whose root is the history of one of the plugin's own entities (`history::history::<Ticket>()`,
    /// `.each_day()` for one row per day).
    pub fn from_history<H: crate::message::Entity + Queryable>(source: history::Source<H>) -> Self {
        const {
            assert!(
                H::PLUGIN.is_none(),
                "a load reads the plugin's own entities' histories only, never another plugin's"
            )
        };
        Load {
            view: View::<E>::from::<H>().history_root(source.each_day),
        }
    }

    /// Joins `S` along a ref between it and a source of the load, either way.
    pub fn join<S: Queryable>(self, r: impl JoinRef<S>) -> Self {
        Load {
            view: self.view.join::<S>(r),
        }
    }

    /// As `join`, keeping the rows that have no `S`.
    pub fn left_join<S: Queryable>(self, r: impl JoinRef<S>) -> Self {
        Load {
            view: self.view.left_join::<S>(r),
        }
    }

    /// Joins `S` on two equal columns (`Stock::sku.eq_col(Price::sku)`).
    pub fn join_on<S: Queryable>(self, on: ColCmp) -> Self {
        Load {
            view: self.view.join_on::<S>(on),
        }
    }

    pub fn left_join_on<S: Queryable>(self, on: ColCmp) -> Self {
        Load {
            view: self.view.left_join_on::<S>(on),
        }
    }

    /// Joins an aliased source (one joined twice).
    pub fn join_as<S: Queryable, N: Nullity>(self, a: &Alias<S, N>, r: impl JoinRef<S>) -> Self {
        Load {
            view: self.view.join_as::<S, N>(a, r),
        }
    }

    /// Keeps the rows for which `cond` holds; several `filter`s are all of them.
    pub fn filter(self, cond: impl Into<Cond>) -> Self {
        Load {
            view: self.view.filter(cond),
        }
    }

    /// Groups the rows by these columns: each must be a column of the load.
    pub fn group_by(self, columns: impl Columns) -> Self {
        Load {
            view: self.view.group_by(columns),
        }
    }

    /// Keeps the groups for which `cond` holds.
    pub fn having(self, cond: impl Into<Cond>) -> Self {
        Load {
            view: self.view.having(cond),
        }
    }

    /// The entity's field `field` is `value`: a column of a source (a bucket of it, a parsed text) or, grouped, an
    /// aggregate, of the field's type.
    pub fn col<V, X: ViewValue>(self, field: Field<E, V>, value: X) -> Self
    where
        X::V: Comparable<V>,
    {
        Load {
            view: self.view.col(field, value),
        }
    }

    /// The load as the descriptor declares it, or the first mistake made building it.
    pub(crate) fn decl(&self) -> std::result::Result<celobus_descriptor::load::Load, String> {
        let v = self.view.decl(String::new())?;
        Ok(celobus_descriptor::load::Load {
            entity: E::ENTITY.to_string(),
            sources: v.sources,
            filter: v.filter,
            group_by: v.group_by,
            having: v.having,
            columns: v.columns,
        })
    }
}

/// A snapshot of the workspace's analytic copy the plugin asks for (`ctx.analytics().snapshot()`): `.load::<E>()`
/// names its entities stored in OLAP whose loads it runs, `.request()` asks.
pub struct Snapshot {
    ctx: Ctx,
    entities: Vec<&'static str>,
    error: Option<String>,
}

impl Snapshot {
    pub(crate) fn new(ctx: Ctx) -> Self {
        Snapshot {
            ctx,
            entities: vec![],
            error: None,
        }
    }

    /// The snapshot runs the load of `E`, one of the plugin's entities stored in OLAP.
    pub fn load<E: crate::message::Entity>(mut self) -> Self {
        if !E::OLAP || E::PLUGIN.is_some() {
            self.error.get_or_insert_with(|| {
                format!(
                    "{} is not one of the plugin's entities stored in OLAP: only those are loaded",
                    E::ENTITY
                )
            });
        }
        if !self.entities.contains(&E::ENTITY) {
            self.entities.push(E::ENTITY);
        }
        self
    }

    /// Asks for the snapshot: answers at once with its [`Ticket`] (a request while the plugin has one waiting is
    /// merged into that one). The snapshot is made in the background; every replica of the workspace then reads from
    /// it. Between two snapshots that load an entity its rows do not change, whatever happens in the lake.
    pub async fn request(self) -> Result<Ticket> {
        if let Some(e) = self.error {
            return Err(Error::new(400, "invalid", e));
        }
        if self.entities.is_empty() {
            return Err(Error::new(
                400,
                "invalid",
                "a snapshot names the entities it loads: .load::<E>()",
            ));
        }
        let r = pb::AnalyticsLoad {
            request_id: self.ctx.request().id(),
            entities: self.entities.iter().map(|e| e.to_string()).collect(),
        };
        match self.ctx.syscall(syscall::Call::AnalyticsLoad(r)).await? {
            Some(syscall_result::Result::AnalyticsTicket(t)) => Ok(Ticket {
                ctx: self.ctx,
                status: TicketStatus::of(t),
            }),
            _ => Err(Error::internal(
                "the platform answered a snapshot's request with something else",
            )),
        }
    }
}

/// Where a ticket stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TicketState {
    /// waiting for the next snapshot
    Pending,
    /// the snapshot that runs its loads is being made or adopted
    Building,
    /// the snapshot that loaded every entity it names is the workspace's, and this plugin's analytic reads answer
    /// from it: a read right after `wait()` sees the loaded rows
    Done,
    /// a load failed (the others of the snapshot stand), or the snapshot could not be made
    Failed,
}

/// One entity's load in a snapshot: its rows and when they were read from the lake, or why it failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    pub entity: String,
    pub rows: u64,
    pub loaded_at: Option<Timestamp>,
    /// empty: loaded; else the failure's code (`analytic_load_rows_limit`, `analytic_load_duplicate`, ...)
    pub code: String,
    pub error: String,
}

/// A ticket as the platform last answered it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TicketStatus {
    pub id: String,
    pub state: TicketState,
    /// the snapshot that ran its loads (0 before)
    pub generation: u64,
    /// the entities it loads
    pub entities: Vec<String>,
    pub loads: Vec<Loaded>,
    /// a failed ticket's first failure
    pub code: String,
    pub error: String,
    pub requested_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
    /// a pending ticket: the earliest its snapshot may begin (snapshots with loads are spaced: the next slot, or now)
    pub not_before: Option<Timestamp>,
}

fn at(ms: i64) -> Option<Timestamp> {
    (ms > 0)
        .then(|| chrono::DateTime::from_timestamp_millis(ms))
        .flatten()
}

impl TicketStatus {
    fn of(t: pb::AnalyticsTicket) -> Self {
        use pb::analytics_ticket::State as S;
        TicketStatus {
            id: t.ticket,
            state: match S::try_from(t.state) {
                Ok(S::Building) => TicketState::Building,
                Ok(S::Done) => TicketState::Done,
                Ok(S::Failed) => TicketState::Failed,
                _ => TicketState::Pending,
            },
            generation: t.generation,
            entities: t.entities,
            loads: t
                .loads
                .into_iter()
                .map(|l| Loaded {
                    entity: l.entity,
                    rows: l.rows,
                    loaded_at: at(l.loaded_at),
                    code: l.code,
                    error: l.error,
                })
                .collect(),
            code: t.code,
            error: t.error,
            requested_at: at(t.requested_at),
            finished_at: at(t.finished_at),
            not_before: at(t.not_before),
        }
    }

    /// A wait that the request's deadline ended before the ticket did: 503 `analytic_load_pending` (retry later; the
    /// ticket goes on), its details the ticket's id, state and `not_before` (unix milliseconds).
    pub fn pending(&self) -> Error {
        let state = match self.state {
            TicketState::Pending => "pending",
            TicketState::Building => "building",
            TicketState::Done => "done",
            TicketState::Failed => "failed",
        };
        Error::new(
            503,
            "analytic_load_pending",
            "the analytic data is still being prepared: the snapshot that loads it has not been made yet",
        )
        .detail("ticket", self.id.clone())
        .detail("state", state)
        .detail(
            "not_before",
            self.not_before.map(|t| t.timestamp_millis()).unwrap_or(0),
        )
    }

    /// The failure as an error: its code, and the status the code answers with.
    pub fn error(&self) -> Option<Error> {
        (self.state == TicketState::Failed).then(|| {
            // a failure without a code is the snapshot's: `analytic_load_failed`, with its status
            let code = if self.code.is_empty() {
                "analytic_load_failed"
            } else {
                self.code.as_str()
            };
            let status = match code {
                "analytic_load_rows_limit"
                | "analytic_load_time_limit"
                | "analytic_load_duplicate"
                | "analytic_load_invalid" => 422,
                "permission_denied" | "capability_missing" => 403,
                "analytic_load_failed" | "unavailable" => 503,
                _ => 500,
            };
            Error::new(status, code.to_string(), self.error.clone())
        })
    }
}

/// How long before a request's deadline a ticket's wait gives up, to answer in time.
const WAIT_MARGIN: std::time::Duration = std::time::Duration::from_secs(2);

/// A request for a snapshot with loads (`Snapshot::request`): its [`TicketStatus`] as answered, [`Ticket::status`]
/// to ask again, [`Ticket::wait`] to wait for its end.
pub struct Ticket {
    ctx: Ctx,
    status: TicketStatus,
}

impl Ticket {
    /// The ticket's id (another call may ask for it: [`Ticket::of`]).
    pub fn id(&self) -> &str {
        &self.status.id
    }

    /// The ticket as last answered.
    pub fn last(&self) -> &TicketStatus {
        &self.status
    }

    /// A ticket of the plugin by its id (one a request answered earlier, perhaps in another call).
    pub fn of(ctx: &Ctx, id: impl Into<String>) -> Ticket {
        Ticket {
            ctx: ctx.clone(),
            status: TicketStatus {
                id: id.into(),
                state: TicketState::Pending,
                generation: 0,
                entities: vec![],
                loads: vec![],
                code: String::new(),
                error: String::new(),
                requested_at: None,
                finished_at: None,
                not_before: None,
            },
        }
    }

    async fn ask(&mut self, wait_ms: u32) -> Result<&TicketStatus> {
        let r = pb::AnalyticsLoadStatus {
            request_id: self.ctx.request().id(),
            ticket: self.status.id.clone(),
            wait_ms,
        };
        match self
            .ctx
            .syscall(syscall::Call::AnalyticsLoadStatus(r))
            .await?
        {
            Some(syscall_result::Result::AnalyticsTicket(t)) => {
                self.status = TicketStatus::of(t);
                Ok(&self.status)
            }
            _ => Err(Error::internal(
                "the platform answered a ticket's state with something else",
            )),
        }
    }

    /// The ticket's state now.
    pub async fn status(&mut self) -> Result<&TicketStatus> {
        self.ask(0).await
    }

    /// Waits until the ticket ends: `Ok` with its state when every load ran, the failure's error otherwise
    /// (`analytic_load_rows_limit`, `analytic_load_duplicate`, ...: the other loads of the snapshot stand). A request
    /// made within ten minutes of the last snapshot with loads waits for the next one: in an endpoint, answer the
    /// ticket and let the caller ask later. In an endpoint the wait ends before the request's deadline: 503
    /// `analytic_load_pending` ([`TicketStatus::pending`]) when the ticket has not ended by then; in a job's run it
    /// waits within the run's timeout.
    pub async fn wait(mut self) -> Result<TicketStatus> {
        let in_run = self.ctx.job().is_run();
        loop {
            let wait_ms = if in_run {
                30_000
            } else {
                // the whole seconds left before the request's deadline, less a margin to answer in
                let left = self
                    .ctx
                    .deadline()
                    .saturating_duration_since(std::time::Instant::now())
                    .saturating_sub(WAIT_MARGIN)
                    .as_millis()
                    .saturating_sub(1)
                    / 1000
                    * 1000;
                if left < 1000 {
                    return Err(self.status.pending());
                }
                left.min(30_000) as u32
            };
            self.ask(wait_ms).await?;
            match self.status.state {
                TicketState::Done => return Ok(self.status),
                TicketState::Failed => {
                    return Err(self
                        .status
                        .error()
                        .unwrap_or_else(|| Error::internal("the load failed")));
                }
                _ => {}
            }
        }
    }
}
