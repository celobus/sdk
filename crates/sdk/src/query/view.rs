//! Analytic views: a named, typed query over the plugin's entities (its dependencies' exported ones, the directory,
//! its other views), declared with `Plugin::view` and created by the platform in both engines. Every row under a view
//! passes its own entity's access rules and masks for the caller who reads it: a view has no rules of its own, and
//! nothing grants reading it but reading what is under it.
//!
//! ```ignore
//! /// One row per paid order line.
//! #[derive(View, Clone, Debug)]
//! #[view(name = "sales")]
//! pub struct Sales {
//!     pub day: Date,
//!     pub clerk: Id<platform::User>,
//!     pub category: String,
//!     pub amount: Decimal,
//! }
//!
//! fn sales() -> View<Sales> {
//!     View::<Sales>::from::<OrderLine>()
//!         .join::<Order>(OrderLine::order_id)
//!         .join::<Product>(OrderLine::product_id)
//!         .filter(Order::status.eq(OrderStatus::Paid))
//!         .col(Sales::day, Order::paid_at.day())
//!         .col(Sales::clerk, Order::created_by)
//!         .col(Sales::category, Product::category)
//!         .col(Sales::amount, OrderLine::amount)
//! }
//! // Plugin::new(..).entity::<Order>().entity::<OrderLine>().entity::<Product>().view(sales())
//!
//! let rows = ctx.analytics().query::<Sales>()                // or ctx.query::<Sales>()
//!     .filter(Sales::day.between(from, to))
//!     .group_by((Sales::day.month(), Sales::category))
//!     .select((Sales::day.month(), Sales::category, sum(Sales::amount)))
//!     .all()
//!     .await?;
//! ```
//!
//! A view's columns are its row type's fields, each given once with `col`: a column of a source (a bucket of a date or
//! a moment too) or, in a grouped view (`group_by`), an aggregate; every grouped column is a column of the view. A
//! view's filter may use [`days_ago`](super::days_ago): it is worked out when the view is read. `celobus plugin
//! check` refuses a view over an unknown entity or column, a join of columns of two types, a column of another type
//! than its field's, views that read each other in a cycle, a view nested more than three deep, and a name an entity
//! of the plugin has.
//!
//! A view's row is no entity: the repository (`ctx.db()`) does not take it, a query does.
//!
//! ```no_run
//! # use celobus_sdk::prelude::*;
//! #[derive(View, Clone, Debug)]
//! pub struct Sales { pub amount: Decimal }
//! # async fn f(ctx: Ctx) -> Result<()> {
//! let rows: Vec<Sales> = ctx.analytics().query::<Sales>().all().await?;
//! # Ok(()) }
//! ```
//!
//! ```compile_fail
//! # use celobus_sdk::prelude::*;
//! #[derive(View, Clone, Debug)]
//! pub struct Sales { pub amount: Decimal }
//! # fn f(ctx: Ctx) {
//! ctx.db::<Sales>();
//! # }
//! ```

use std::marker::PhantomData;

use celobus_descriptor::view as dv;
use celobus_proto as pb;
use pb::{db_condition::Op, query_aggregate::Fn as AggFn, query_expr::Node, query_term};

use super::terms::{
    Agg, Alias, ColCmp, Column, Columns, Comparable, Cond, JoinRef, Nullity, Queryable, canonical,
    default_alias,
};
use crate::message::Message;
use crate::ui::Field;

/// An analytic view of rows `V` (`#[derive(View)]`), as its query builds it.
pub struct View<V> {
    tree: pb::Query,
    error: Option<String>,
    columns: Vec<(&'static str, pb::QuerySelect)>,
    _v: PhantomData<fn() -> V>,
}

/// What a view's column is: a column of a source (or its bucket), or an aggregate.
pub trait ViewValue {
    type V;
    #[doc(hidden)]
    fn select(&self) -> pb::QuerySelect;
}

impl<C: Column> ViewValue for C {
    type V = C::V;
    fn select(&self) -> pb::QuerySelect {
        pb::QuerySelect {
            kind: Some(pb::query_select::Kind::Column(self.wire())),
        }
    }
}

impl<O> ViewValue for Agg<O> {
    type V = O;
    fn select(&self) -> pb::QuerySelect {
        pb::QuerySelect {
            kind: Some(pb::query_select::Kind::Aggregate(self.wire.clone())),
        }
    }
}

impl<V: Queryable + Message> View<V> {
    /// A view whose root is `E` (an entity of the plugin, a dependency's exported entity, the directory's, or another
    /// view of the plugin).
    pub fn from<E: Queryable>() -> Self {
        View {
            tree: pb::Query {
                sources: vec![pb::QuerySource {
                    entity: canonical::<E>(),
                    alias: default_alias::<E>(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            error: None,
            columns: vec![],
            _v: PhantomData,
        }
    }

    /// The root is the entity's history instead of its rows (`each_day`: per day): a model of the lake's.
    pub(crate) fn history_root(mut self, each_day: bool) -> Self {
        if let Some(root) = self.tree.sources.first_mut() {
            root.history = true;
            root.each_day = each_day;
        }
        self
    }

    /// Joins `T` along a ref between it and a source of the view, either way.
    pub fn join<T: Queryable>(mut self, r: impl JoinRef<T>) -> Self {
        super::add_join(
            &mut self.tree,
            &mut self.error,
            canonical::<T>(),
            default_alias::<T>(),
            false,
            r.join_ref(),
        );
        self
    }

    /// As `join`, keeping the rows that have no `T` (or none the caller may read): `T`'s columns are null there.
    pub fn left_join<T: Queryable>(mut self, r: impl JoinRef<T>) -> Self {
        super::add_join(
            &mut self.tree,
            &mut self.error,
            canonical::<T>(),
            default_alias::<T>(),
            true,
            r.join_ref(),
        );
        self
    }

    /// Joins `T` on two equal columns (`Product::sku.eq_col(OrderLine::sku)`): one entity's keys, or two plain
    /// columns of one type.
    pub fn join_on<T: Queryable>(mut self, on: ColCmp) -> Self {
        super::add_join_on(
            &mut self.tree,
            &mut self.error,
            canonical::<T>(),
            default_alias::<T>(),
            false,
            on,
        );
        self
    }

    pub fn left_join_on<T: Queryable>(mut self, on: ColCmp) -> Self {
        super::add_join_on(
            &mut self.tree,
            &mut self.error,
            canonical::<T>(),
            default_alias::<T>(),
            true,
            on,
        );
        self
    }

    /// Joins an aliased entity (an entity joined twice), inner or left as the alias was made.
    pub fn join_as<T: Queryable, N: Nullity>(
        mut self,
        a: &Alias<T, N>,
        r: impl JoinRef<T>,
    ) -> Self {
        super::add_join(
            &mut self.tree,
            &mut self.error,
            canonical::<T>(),
            a.name.clone(),
            N::LEFT,
            r.join_ref(),
        );
        self
    }

    /// Keeps the rows for which `cond` holds; several `filter`s are all of them.
    pub fn filter(mut self, cond: impl Into<Cond>) -> Self {
        let c = Cond(self.tree.filter.take()) & cond.into();
        self.tree.filter = c.0;
        self
    }

    /// Groups the rows by these columns: each must be a column of the view.
    pub fn group_by(mut self, columns: impl Columns) -> Self {
        self.tree.group_by.extend(columns.columns());
        self
    }

    /// Keeps the groups for which `cond` holds.
    pub fn having(mut self, cond: impl Into<Cond>) -> Self {
        let c = Cond(self.tree.having.take()) & cond.into();
        self.tree.having = c.0;
        self
    }

    /// The view's column `field` is `value`: a column of a source (`Order::created_by`, `Order::paid_at.day()`) or,
    /// grouped, an aggregate (`sum(OrderLine::amount)`), of the field's type.
    pub fn col<T, X: ViewValue>(mut self, field: Field<V, T>, value: X) -> Self
    where
        X::V: Comparable<T>,
    {
        if self.columns.iter().any(|(n, _)| *n == field.name()) {
            if self.error.is_none() {
                self.error = Some(format!("the column {} is given twice", field.name()));
            }
            return self;
        }
        self.columns.push((field.name(), value.select()));
        self
    }

    /// The view's name.
    pub fn name(&self) -> &'static str {
        V::SOURCE
    }

    /// The view as the descriptor declares it (`description`: its row type's doc), or the first mistake made
    /// building it.
    pub(crate) fn decl(&self, description: String) -> Result<dv::View, String> {
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        let tree = &self.tree;
        Ok(dv::View {
            name: V::SOURCE.to_string(),
            message: <V as Message>::NAME.to_string(),
            description,
            sources: tree
                .sources
                .iter()
                .map(|s| dv::ViewSource {
                    entity: s.entity.clone(),
                    alias: s.alias.clone(),
                    join: match pb::query_source::Join::try_from(s.join) {
                        Ok(pb::query_source::Join::Left) => "left".into(),
                        Ok(pb::query_source::Join::Inner) => "inner".into(),
                        _ => String::new(),
                    },
                    left: s.left.as_ref().map(column),
                    right: s.right.as_ref().map(column),
                    history: s.history || s.each_day,
                    each_day: s.each_day,
                })
                .collect(),
            filter: tree.filter.as_ref().map(expr).transpose()?,
            group_by: tree.group_by.iter().map(column).collect(),
            having: tree.having.as_ref().map(expr).transpose()?,
            columns: self
                .columns
                .iter()
                .map(|(name, s)| {
                    Ok(dv::ViewColumn {
                        name: name.to_string(),
                        value: match &s.kind {
                            Some(pb::query_select::Kind::Column(c)) => dv::Term::Column(column(c)),
                            Some(pb::query_select::Kind::Aggregate(a)) => {
                                dv::Term::Aggregate(aggregate(a))
                            }
                            _ => return Err(format!("{name}: not a column")),
                        },
                    })
                })
                .collect::<Result<_, String>>()?,
        })
    }
}

pub(crate) fn column(c: &pb::QueryColumn) -> dv::Column {
    use pb::query_column::{Bucket, Parse};
    dv::Column {
        parse: match Parse::try_from(c.parse) {
            Ok(Parse::Decimal) => "decimal",
            Ok(Parse::Int32) => "int32",
            Ok(Parse::Int64) => "int64",
            Ok(Parse::Double) => "double",
            Ok(Parse::Bool) => "bool",
            Ok(Parse::Date) => "date",
            Ok(Parse::Timestamp) => "timestamp",
            Ok(Parse::Uuid) => "uuid",
            _ => "",
        }
        .into(),
        source: c.source.clone(),
        field: c.field.clone(),
        bucket: match Bucket::try_from(c.bucket) {
            Ok(Bucket::Day) => "day",
            Ok(Bucket::Week) => "week",
            Ok(Bucket::Month) => "month",
            Ok(Bucket::Quarter) => "quarter",
            Ok(Bucket::Year) => "year",
            _ => "",
        }
        .into(),
    }
}

fn aggregate(a: &pb::QueryAggregate) -> dv::Aggregate {
    dv::Aggregate {
        func: match AggFn::try_from(a.r#fn) {
            Ok(AggFn::CountDistinct) => "count_distinct",
            Ok(AggFn::Sum) => "sum",
            Ok(AggFn::Avg) => "avg",
            Ok(AggFn::Min) => "min",
            Ok(AggFn::Max) => "max",
            _ => "count",
        }
        .into(),
        column: a.column.as_ref().map(column),
    }
}

fn term(t: &pb::QueryTerm) -> Result<dv::Term, String> {
    Ok(match &t.kind {
        Some(query_term::Kind::Column(c)) => dv::Term::Column(column(c)),
        Some(query_term::Kind::Aggregate(a)) => dv::Term::Aggregate(aggregate(a)),
        Some(query_term::Kind::DaysAgo(n)) => dv::Term::DaysAgo(*n),
        None => return Err("a term is missing".into()),
    })
}

fn op(o: i32) -> Result<&'static str, String> {
    Ok(match Op::try_from(o) {
        Ok(Op::Eq) => "eq",
        Ok(Op::Ne) => "ne",
        Ok(Op::Lt) => "lt",
        Ok(Op::Le) => "le",
        Ok(Op::Gt) => "gt",
        Ok(Op::Ge) => "ge",
        Ok(Op::In) => "in",
        Ok(Op::IsNull) => "is_null",
        Ok(Op::IsNotNull) => "is_not_null",
        Ok(Op::Search) => "search",
        Ok(Op::Contains) => "contains",
        _ => return Err("a condition has no operator".into()),
    })
}

fn value(v: &pb::DbValue) -> serde_json::Value {
    use pb::db_value::Kind;
    match &v.kind {
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(Kind::IntValue(i)) => serde_json::Value::from(*i),
        Some(Kind::DoubleValue(d)) => serde_json::Value::from(*d),
        _ => serde_json::Value::Null,
    }
}

fn expr(e: &pb::QueryExpr) -> Result<dv::Expr, String> {
    let list = |l: &pb::QueryExprs| l.items.iter().map(expr).collect::<Result<Vec<_>, _>>();
    Ok(match &e.node {
        Some(Node::And(l)) => dv::Expr::And(list(l)?),
        Some(Node::Or(l)) => dv::Expr::Or(list(l)?),
        Some(Node::Not(x)) => dv::Expr::Not(Box::new(expr(x)?)),
        Some(Node::Compare(c)) => dv::Expr::Compare(dv::Compare {
            left: term(c.left.as_ref().ok_or("a comparison has no left side")?)?,
            op: op(c.op)?.into(),
            values: c.values.iter().map(value).collect(),
            right: c.right.as_ref().map(term).transpose()?,
        }),
        Some(Node::Exists(x)) => {
            let s = x.source.as_ref().ok_or("exists names no entity")?;
            dv::Expr::Exists(dv::Exists {
                entity: s.entity.clone(),
                alias: s.alias.clone(),
                filter: x.filter.as_deref().map(expr).transpose()?.map(Box::new),
            })
        }
        None => dv::Expr::And(vec![]),
    })
}
