//! The terms of a query: the entities it may name ([`Queryable`]), columns (field handles, and an alias's columns),
//! conditions ([`Cond`]: the field handles' filters, a column against a column, `exists`), aggregates, labels and
//! the refs a join follows.

use std::marker::PhantomData;
use std::ops::{BitAnd, BitOr, Not};

use celobus_proto as pb;
use pb::{db_condition::Op, db_filter, query_aggregate::Fn as AggFn, query_expr::Node, query_term};
use serde_json::Value;

use crate::db::{Filter, Order};
use crate::error::Result;
use crate::message::{Entity, FieldValue};
use crate::types::Id;
use crate::ui::Field;

/// An entity a query may name: the plugin's own (`#[derive(Entity)]`), a dependency's whose provider exports `read`
/// (the type `celobus plugin add` generates for it, and only for it), the directory's (`platform::User`, `Group`,
/// `GroupMember`, `RoleAssignment`). Implemented by the derive and the SDK, never by hand.
pub trait Queryable: Entity {}

/// The entity's name in a query: `order` (the plugin's own), `crm.customer`, `platform.user`.
pub fn canonical<E: Entity>() -> String {
    match E::PLUGIN {
        None => E::ENTITY.to_string(),
        Some(p) => format!("{p}.{}", E::ENTITY),
    }
}

/// The alias an entity has in a query unless it is given one: its name with `_` for `.` (`crm_customer`).
pub fn default_alias<E: Entity>() -> String {
    canonical::<E>().replace('.', "_")
}

/// A value type and what it is when its column may come back null (a left join): `Option<T>`, and an `Option<T>`
/// as it is (one level). Implemented by the SDK for its types and by the derives.
pub trait Nullable: FieldValue {
    type Opt: FieldValue;
}

macro_rules! nullable {
    ($($t:ty),*) => { $( impl Nullable for $t { type Opt = Option<$t>; } )* };
}
nullable!(
    String,
    bool,
    i32,
    i64,
    u8,
    f64,
    crate::types::Decimal,
    crate::types::Uuid,
    crate::types::Timestamp,
    crate::types::Date
);
impl<T: FieldValue> Nullable for Option<T> {
    type Opt = Option<T>;
}
impl<T: FieldValue> Nullable for Vec<T> {
    type Opt = Option<Vec<T>>;
}
impl<E: Entity> Nullable for Id<E> {
    type Opt = Option<Id<E>>;
}
impl<T: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static> Nullable
    for crate::types::Json<T>
{
    type Opt = Option<crate::types::Json<T>>;
}

/// Whether a source's columns come back as they are ([`Req`]) or may be null ([`Opt`]: a left join).
pub trait Nullity: 'static {
    type Wrap<T: Nullable>: FieldValue;
    const LEFT: bool;
}

/// An entity joined inner (or the root).
pub struct Req;
/// An entity joined left: its columns come back as `Option<T>`.
pub struct Opt;

impl Nullity for Req {
    type Wrap<T: Nullable> = T;
    const LEFT: bool = false;
}

impl Nullity for Opt {
    type Wrap<T: Nullable> = T::Opt;
    const LEFT: bool = true;
}

/// A type at the type level only.
type Marker<T> = fn() -> T;

pub(crate) fn column(source: &str, field: &str) -> pb::QueryColumn {
    pb::QueryColumn {
        source: source.into(),
        field: field.into(),
    }
}

/// A column of a query: a field handle (`Order::status`, of the entity's default alias) or an alias's column
/// (`manager.col(platform::User::display_name)`); `V` is its value type.
pub trait Column {
    type Entity: Entity;
    type V: FieldValue;
    #[doc(hidden)]
    fn wire(&self) -> pb::QueryColumn;
}

impl<E: Queryable, V: FieldValue> Column for Field<E, V> {
    type Entity = E;
    type V = V;
    fn wire(&self) -> pb::QueryColumn {
        column(&default_alias::<E>(), self.name())
    }
}

/// A column of an aliased entity (see [`alias`]).
pub struct ACol<E, V, N> {
    pub(crate) alias: String,
    pub(crate) field: &'static str,
    _p: PhantomData<Marker<(E, V, N)>>,
}

impl<E, V, N> Clone for ACol<E, V, N> {
    fn clone(&self) -> Self {
        Self {
            alias: self.alias.clone(),
            field: self.field,
            _p: PhantomData,
        }
    }
}

impl<E: Queryable, V: FieldValue, N> Column for ACol<E, V, N> {
    type Entity = E;
    type V = V;
    fn wire(&self) -> pb::QueryColumn {
        column(&self.alias, self.field)
    }
}

/// An entity under a name of its own, for an entity joined twice: `alias::<platform::User>("manager")` (inner) or
/// `.left()`; join it with `join_as`, name its columns with `col`.
pub struct Alias<T, N = Req> {
    pub(crate) name: String,
    _p: PhantomData<fn() -> (T, N)>,
}

/// An alias of `T` (`[a-z][a-z0-9_]*`).
pub fn alias<T: Queryable>(name: &str) -> Alias<T, Req> {
    Alias {
        name: name.into(),
        _p: PhantomData,
    }
}

impl<T: Queryable> Alias<T, Req> {
    /// The alias joined left: its columns come back as `Option<V>`.
    pub fn left(self) -> Alias<T, Opt> {
        Alias {
            name: self.name,
            _p: PhantomData,
        }
    }
}

impl<T: Queryable, N: Nullity> Alias<T, N> {
    /// The alias's column of a field of `T`.
    pub fn col<V: FieldValue>(&self, f: Field<T, V>) -> ACol<T, V, N> {
        ACol {
            alias: self.name.clone(),
            field: f.name(),
            _p: PhantomData,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Two value types that compare: the same, or one of them optional.
pub trait Comparable<T> {}
impl<T> Comparable<T> for T {}
impl<T> Comparable<Option<T>> for T {}
impl<T> Comparable<T> for Option<T> {}

/// A condition of a query: built from the field handles' filters (`Order::status.eq(..)`), a column against a column
/// (`a.eq_col(b)`), `exists`, combined with `&`, `|`, `!`. Filters on one entity combine as `Filter<E>`; across
/// entities, combine them as `Cond` (`Cond::from(a) & b`) or give `filter` each.
#[derive(Clone, Debug, Default)]
pub struct Cond(pub(crate) Option<pb::QueryExpr>);

impl Cond {
    /// Every row.
    pub fn all() -> Self {
        Cond(None)
    }

    pub(crate) fn expr(node: Node) -> Self {
        Cond(Some(pb::QueryExpr { node: Some(node) }))
    }

    fn combine(self, other: Cond, and: bool) -> Cond {
        match (self.0, other.0) {
            (None, x) | (x, None) if and => Cond(x),
            (None, _) | (_, None) => Cond::all(),
            (Some(a), Some(b)) => {
                let items = pb::QueryExprs { items: vec![a, b] };
                Cond::expr(if and {
                    Node::And(items)
                } else {
                    Node::Or(items)
                })
            }
        }
    }
}

fn from_filter(alias: &str, f: &pb::DbFilter) -> pb::QueryExpr {
    let node = match &f.node {
        None => None,
        Some(db_filter::Node::And(l)) => Some(Node::And(pb::QueryExprs {
            items: l.items.iter().map(|i| from_filter(alias, i)).collect(),
        })),
        Some(db_filter::Node::Or(l)) => Some(Node::Or(pb::QueryExprs {
            items: l.items.iter().map(|i| from_filter(alias, i)).collect(),
        })),
        Some(db_filter::Node::Not(i)) => Some(Node::Not(Box::new(from_filter(alias, i)))),
        Some(db_filter::Node::Condition(c)) => Some(Node::Compare(pb::QueryCompare {
            left: Some(pb::QueryTerm {
                kind: Some(query_term::Kind::Column(column(alias, &c.field))),
            }),
            op: c.op,
            values: c.values.clone(),
            right: None,
        })),
    };
    pb::QueryExpr { node }
}

impl<E: Queryable> From<Filter<E>> for Cond {
    fn from(f: Filter<E>) -> Self {
        Cond(f.node().map(|n| from_filter(&default_alias::<E>(), n)))
    }
}

impl<R: Into<Cond>> BitAnd<R> for Cond {
    type Output = Cond;
    fn bitand(self, rhs: R) -> Cond {
        self.combine(rhs.into(), true)
    }
}

impl<R: Into<Cond>> BitOr<R> for Cond {
    type Output = Cond;
    fn bitor(self, rhs: R) -> Cond {
        self.combine(rhs.into(), false)
    }
}

impl Not for Cond {
    type Output = Cond;
    fn not(self) -> Cond {
        match self.0 {
            None => Cond::expr(Node::Or(pb::QueryExprs { items: vec![] })),
            Some(e) => Cond::expr(Node::Not(Box::new(e))),
        }
    }
}

impl<E: Queryable> BitAnd<Cond> for Filter<E> {
    type Output = Cond;
    fn bitand(self, rhs: Cond) -> Cond {
        Cond::from(self) & rhs
    }
}

impl<E: Queryable> BitOr<Cond> for Filter<E> {
    type Output = Cond;
    fn bitor(self, rhs: Cond) -> Cond {
        Cond::from(self) | rhs
    }
}

/// A comparison of two columns: a condition, and what `join_on` joins on (`eq_col`).
#[derive(Clone, Debug)]
pub struct ColCmp {
    pub(crate) left: pb::QueryColumn,
    pub(crate) op: Op,
    pub(crate) right: pb::QueryColumn,
}

impl From<ColCmp> for Cond {
    fn from(c: ColCmp) -> Cond {
        let term = |c: pb::QueryColumn| pb::QueryTerm {
            kind: Some(query_term::Kind::Column(c)),
        };
        Cond::expr(Node::Compare(pb::QueryCompare {
            left: Some(term(c.left)),
            op: c.op as i32,
            values: vec![],
            right: Some(term(c.right)),
        }))
    }
}

impl<R: Into<Cond>> BitAnd<R> for ColCmp {
    type Output = Cond;
    fn bitand(self, rhs: R) -> Cond {
        Cond::from(self) & rhs
    }
}

impl<R: Into<Cond>> BitOr<R> for ColCmp {
    type Output = Cond;
    fn bitor(self, rhs: R) -> Cond {
        Cond::from(self) | rhs
    }
}

impl Not for ColCmp {
    type Output = Cond;
    fn not(self) -> Cond {
        !Cond::from(self)
    }
}

/// A column compared with a column of the same type (`Id<E>` with `Id<E>`, `String` with `Option<String>`): a
/// condition, or (`eq_col`) what `join_on` joins on.
pub trait ColumnCompare: Column + Sized {
    fn cmp_col<C: Column>(&self, op: Op, other: C) -> ColCmp
    where
        C::V: Comparable<Self::V>,
    {
        ColCmp {
            left: self.wire(),
            op,
            right: other.wire(),
        }
    }

    fn eq_col<C: Column>(&self, other: C) -> ColCmp
    where
        C::V: Comparable<Self::V>,
    {
        self.cmp_col(Op::Eq, other)
    }

    /// Not equal (a null is not equal to a value).
    fn ne_col<C: Column>(&self, other: C) -> ColCmp
    where
        C::V: Comparable<Self::V>,
    {
        self.cmp_col(Op::Ne, other)
    }

    fn lt_col<C: Column>(&self, other: C) -> ColCmp
    where
        C::V: Comparable<Self::V>,
    {
        self.cmp_col(Op::Lt, other)
    }

    fn le_col<C: Column>(&self, other: C) -> ColCmp
    where
        C::V: Comparable<Self::V>,
    {
        self.cmp_col(Op::Le, other)
    }

    fn gt_col<C: Column>(&self, other: C) -> ColCmp
    where
        C::V: Comparable<Self::V>,
    {
        self.cmp_col(Op::Gt, other)
    }

    fn ge_col<C: Column>(&self, other: C) -> ColCmp
    where
        C::V: Comparable<Self::V>,
    {
        self.cmp_col(Op::Ge, other)
    }
}

impl<T: Column> ColumnCompare for T {}

impl<E: Queryable, V: FieldValue, N> ACol<E, V, N> {
    fn compare(&self, op: Op, values: Vec<Value>) -> Cond {
        Cond::expr(Node::Compare(pb::QueryCompare {
            left: Some(pb::QueryTerm {
                kind: Some(query_term::Kind::Column(self.wire())),
            }),
            op: op as i32,
            values: values.into_iter().map(crate::db::db_value).collect(),
            right: None,
        }))
    }

    pub fn eq(&self, v: V) -> Cond {
        self.compare(Op::Eq, vec![v.encode()])
    }
    pub fn ne(&self, v: V) -> Cond {
        self.compare(Op::Ne, vec![v.encode()])
    }
    pub fn lt(&self, v: V) -> Cond {
        self.compare(Op::Lt, vec![v.encode()])
    }
    pub fn le(&self, v: V) -> Cond {
        self.compare(Op::Le, vec![v.encode()])
    }
    pub fn gt(&self, v: V) -> Cond {
        self.compare(Op::Gt, vec![v.encode()])
    }
    pub fn ge(&self, v: V) -> Cond {
        self.compare(Op::Ge, vec![v.encode()])
    }
    pub fn is_in(&self, values: impl IntoIterator<Item = V>) -> Cond {
        self.compare(Op::In, values.into_iter().map(|v| v.encode()).collect())
    }
    pub fn is_null(&self) -> Cond {
        self.compare(Op::IsNull, vec![])
    }
    pub fn is_not_null(&self) -> Cond {
        self.compare(Op::IsNotNull, vec![])
    }
    pub fn asc(&self) -> QOrder {
        QOrder::column(self.wire(), false)
    }
    pub fn desc(&self) -> QOrder {
        QOrder::column(self.wire(), true)
    }
}

/// A row of `T` for which `cond` holds exists (`cond` may name the query's entities: it is correlated).
pub fn exists<T: Queryable>(cond: impl Into<Cond>) -> Cond {
    exists_named(canonical::<T>(), default_alias::<T>(), cond.into())
}

/// No row of `T` for which `cond` holds exists.
pub fn not_exists<T: Queryable>(cond: impl Into<Cond>) -> Cond {
    !exists::<T>(cond)
}

/// `exists` of an aliased entity (`exists_as(&other, other.col(Order::created_by).eq_col(Order::created_by))`).
pub fn exists_as<T: Queryable, N>(a: &Alias<T, N>, cond: impl Into<Cond>) -> Cond {
    exists_named(canonical::<T>(), a.name.clone(), cond.into())
}

fn exists_named(entity: String, alias: String, cond: Cond) -> Cond {
    Cond::expr(Node::Exists(Box::new(pb::QueryExists {
        source: Some(pb::QuerySource {
            entity,
            alias,
            ..Default::default()
        }),
        filter: cond.0.map(Box::new),
    })))
}

/// An order of a query: a field handle's (`Order::total.desc()`), an alias's column's, an aggregate's
/// (`sum(OrderLine::amount).desc()`).
#[derive(Clone, Debug)]
pub struct QOrder(pub(crate) pb::QueryOrder);

impl QOrder {
    fn column(c: pb::QueryColumn, desc: bool) -> Self {
        QOrder(pb::QueryOrder {
            term: Some(pb::QueryTerm {
                kind: Some(query_term::Kind::Column(c)),
            }),
            desc,
        })
    }
}

impl<E: Queryable> From<Order<E>> for QOrder {
    fn from(o: Order<E>) -> Self {
        QOrder::column(column(&default_alias::<E>(), o.field()), o.is_desc())
    }
}

/// An aggregate whose value is an `O`: `count()`, `sum(..)` and the others; selected, ordered by, or compared in
/// `having`.
pub struct Agg<O> {
    pub(crate) wire: pb::QueryAggregate,
    _o: PhantomData<fn() -> O>,
}

impl<O> Clone for Agg<O> {
    fn clone(&self) -> Self {
        Self {
            wire: self.wire.clone(),
            _o: PhantomData,
        }
    }
}

fn agg<O>(f: AggFn, c: Option<pb::QueryColumn>) -> Agg<O> {
    Agg {
        wire: pb::QueryAggregate {
            r#fn: f as i32,
            column: c,
        },
        _o: PhantomData,
    }
}

/// The rows (of a group).
pub fn count() -> Agg<i64> {
    agg(AggFn::Count, None)
}

/// The distinct values of a column (nulls not counted).
pub fn count_distinct<C: Column>(c: C) -> Agg<i64> {
    agg(AggFn::CountDistinct, Some(c.wire()))
}

/// The sum of a number column: `i32` and `i64` sum to an `i64`, `f64` to an `f64`, `Decimal` to a `Decimal` (0
/// when there is none).
pub fn sum<C: Column>(c: C) -> Agg<<C::V as crate::db::Summable>::Sum>
where
    C::V: crate::db::Summable,
{
    agg(AggFn::Sum, Some(c.wire()))
}

/// The average of a number column: a `Decimal` of integers and decimals, an `f64` of doubles (`None` when every
/// value is null).
pub fn avg<C: Column>(c: C) -> Agg<Option<<C::V as Averaged>::Avg>>
where
    C::V: Averaged,
{
    agg(AggFn::Avg, Some(c.wire()))
}

/// The smallest value of a column (`None` when there is none).
pub fn min<C: Column>(c: C) -> Agg<<C::V as Nullable>::Opt>
where
    C::V: Nullable,
{
    agg(AggFn::Min, Some(c.wire()))
}

/// The largest value of a column (`None` when there is none).
pub fn max<C: Column>(c: C) -> Agg<<C::V as Nullable>::Opt>
where
    C::V: Nullable,
{
    agg(AggFn::Max, Some(c.wire()))
}

/// A number column's average type.
pub trait Averaged: FieldValue {
    type Avg: FieldValue;
}
impl Averaged for i32 {
    type Avg = crate::types::Decimal;
}
impl Averaged for i64 {
    type Avg = crate::types::Decimal;
}
impl Averaged for crate::types::Decimal {
    type Avg = crate::types::Decimal;
}
impl Averaged for f64 {
    type Avg = f64;
}
impl<T: Averaged> Averaged for Option<T> {
    type Avg = T::Avg;
}

impl<O: FieldValue> Agg<O> {
    fn compare(&self, op: Op, v: Value) -> Cond {
        Cond::expr(Node::Compare(pb::QueryCompare {
            left: Some(pb::QueryTerm {
                kind: Some(query_term::Kind::Aggregate(self.wire.clone())),
            }),
            op: op as i32,
            values: vec![crate::db::db_value(v)],
            right: None,
        }))
    }

    pub fn eq(&self, v: impl Into<O>) -> Cond {
        self.compare(Op::Eq, v.into().encode())
    }
    pub fn ne(&self, v: impl Into<O>) -> Cond {
        self.compare(Op::Ne, v.into().encode())
    }
    pub fn lt(&self, v: impl Into<O>) -> Cond {
        self.compare(Op::Lt, v.into().encode())
    }
    pub fn le(&self, v: impl Into<O>) -> Cond {
        self.compare(Op::Le, v.into().encode())
    }
    pub fn gt(&self, v: impl Into<O>) -> Cond {
        self.compare(Op::Gt, v.into().encode())
    }
    pub fn ge(&self, v: impl Into<O>) -> Cond {
        self.compare(Op::Ge, v.into().encode())
    }

    pub fn asc(&self) -> QOrder {
        self.order(false)
    }

    pub fn desc(&self) -> QOrder {
        self.order(true)
    }

    fn order(&self, desc: bool) -> QOrder {
        QOrder(pb::QueryOrder {
            term: Some(pb::QueryTerm {
                kind: Some(query_term::Kind::Aggregate(self.wire.clone())),
            }),
            desc,
        })
    }
}

/// A row's label, as `label(ref)` selects it: a person's (or a group's) name and avatar, another row's label column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub id: String,
    pub text: String,
    pub avatar_url: Option<String>,
}

impl Label {
    pub(crate) fn decode(v: &Value) -> Result<Option<Label>> {
        let Some(o) = v.as_object() else {
            return Ok(None);
        };
        let s = |k: &str| o.get(k).and_then(Value::as_str).map(str::to_string);
        Ok(Some(Label {
            id: s("id").unwrap_or_default(),
            text: s("text").unwrap_or_default(),
            avatar_url: s("avatarUrl"),
        }))
    }
}

/// A column that names a row: a ref (`Id<T>`, `Option<Id<T>>`) or an `id`.
pub trait RefValue: FieldValue {}
impl<T: Entity> RefValue for Id<T> {}
impl<T: Entity> RefValue for Option<Id<T>> {}

/// `label(c)`: the label of the row a ref names (or of the row of an `id`), for every reader of that row; for a
/// person their name and avatar, though the directory itself gives a guest no rows. `None` when the ref is null or
/// names a row the caller may not read.
pub fn label<C: Column>(c: C) -> LabelOf
where
    C::V: RefValue,
{
    LabelOf(c.wire())
}

/// A `label(..)` select item.
#[derive(Clone, Debug)]
pub struct LabelOf(pub(crate) pb::QueryColumn);

/// A ref a join follows between `T` and an entity of the query: `Order::customer_id` joins `Customer` to a query
/// that has `Order` (forward) or `Order` to one that has `Customer` (reverse). Implemented for every ref field: a
/// join on a ref between other entities does not compile.
pub trait JoinRef<T> {
    /// (the entity whose column the ref is, the column, the entity it names), by their names in a query
    #[doc(hidden)]
    fn join_ref(&self) -> (String, &'static str, String);
}

impl<X: Entity, T: Entity> JoinRef<T> for Field<X, Id<T>> {
    fn join_ref(&self) -> (String, &'static str, String) {
        (canonical::<X>(), self.name(), canonical::<T>())
    }
}

impl<X: Entity, T: Entity> JoinRef<T> for Field<X, Option<Id<T>>> {
    fn join_ref(&self) -> (String, &'static str, String) {
        (canonical::<X>(), self.name(), canonical::<T>())
    }
}

/// The columns `group_by` takes: one column, or a tuple of them.
pub trait Columns {
    #[doc(hidden)]
    fn columns(&self) -> Vec<pb::QueryColumn>;
}

impl<C: Column> Columns for C {
    fn columns(&self) -> Vec<pb::QueryColumn> {
        vec![self.wire()]
    }
}

macro_rules! columns_tuple {
    ($($n:ident),+) => {
        impl<$($n: Column),+> Columns for ($($n,)+) {
            #[allow(non_snake_case)]
            fn columns(&self) -> Vec<pb::QueryColumn> {
                let ($($n,)+) = self;
                vec![$($n.wire()),+]
            }
        }
    };
}
columns_tuple!(A);
columns_tuple!(A, B);
columns_tuple!(A, B, C);
columns_tuple!(A, B, C, D);
columns_tuple!(A, B, C, D, E);
columns_tuple!(A, B, C, D, E, F);
columns_tuple!(A, B, C, D, E, F, G);
columns_tuple!(A, B, C, D, E, F, G, H);
