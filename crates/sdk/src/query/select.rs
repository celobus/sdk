//! What a query selects, typed: a column (its value type, or `Option` of it when its entity is joined left), an
//! aggregate, a label; a tuple of them is a row of a tuple. The type of a field handle's column is found by its
//! entity's place in the query's sources (`Has`), so selecting a column of an entity the query does not have does not
//! compile.

use std::marker::PhantomData;

use celobus_proto as pb;
use pb::query_select;
use serde_json::Value;

use super::terms::{
    ACol, Agg, Column, Label, LabelOf, Nullable, Nullity, Queryable, column, default_alias,
};
use crate::error::{Error, Result};
use crate::message::FieldValue;
use crate::ui::Field;

/// The sources of a query at the type level: `Cons<Entity, Req | Opt, Rest>`, ending in `Nil`.
pub struct Nil;
/// An entity `E` of the sources, joined as `N`, before the rest `R`.
pub struct Cons<E, N, R>(PhantomData<Marker<(E, N, R)>>);

/// A type at the type level only.
type Marker<T> = fn() -> T;

/// Where an entity is in the sources (inferred).
pub struct Here;
pub struct There<I>(PhantomData<fn() -> I>);

/// The query's sources have `E`, joined as `N`.
pub trait Has<E, I> {
    type N: Nullity;
}

impl<E, N: Nullity, R> Has<E, Here> for Cons<E, N, R> {
    type N = N;
}

impl<E, F, N, R: Has<E, I>, I> Has<E, There<I>> for Cons<F, N, R> {
    type N = R::N;
}

/// A value as a select item reads it: null is the type's missing value (`None`, or its default).
pub(crate) fn value<T: FieldValue>(v: &Value, path: &str) -> Result<T> {
    if v.is_null() {
        T::missing(path)
    } else {
        T::decode(v, path)
    }
}

/// An item of a select over the sources `S`.
pub trait Item<S, I> {
    type Out;
    #[doc(hidden)]
    fn item(&self) -> pb::QuerySelect;
    #[doc(hidden)]
    fn read(v: &Value, path: &str) -> Result<Self::Out>;
}

fn select(kind: query_select::Kind) -> pb::QuerySelect {
    pb::QuerySelect { kind: Some(kind) }
}

impl<S, I, E: Queryable, T: Nullable> Item<S, I> for Field<E, T>
where
    S: Has<E, I>,
{
    type Out = <<S as Has<E, I>>::N as Nullity>::Wrap<T>;
    fn item(&self) -> pb::QuerySelect {
        select(query_select::Kind::Column(column(
            &default_alias::<E>(),
            self.name(),
        )))
    }
    fn read(v: &Value, path: &str) -> Result<Self::Out> {
        value(v, path)
    }
}

impl<S, I, E: Queryable, T: Nullable> Item<S, I> for super::terms::Bucket<E, T>
where
    S: Has<E, I>,
{
    type Out = <<S as Has<E, I>>::N as Nullity>::Wrap<T>;
    fn item(&self) -> pb::QuerySelect {
        select(query_select::Kind::Column(self.wire()))
    }
    fn read(v: &Value, path: &str) -> Result<Self::Out> {
        value(v, path)
    }
}

/// The index of an item that does not depend on the sources.
pub struct Free;

impl<S, E: Queryable, T: Nullable, N: Nullity> Item<S, Free> for ACol<E, T, N> {
    type Out = N::Wrap<T>;
    fn item(&self) -> pb::QuerySelect {
        select(query_select::Kind::Column(self.wire()))
    }
    fn read(v: &Value, path: &str) -> Result<Self::Out> {
        value(v, path)
    }
}

impl<S, O: FieldValue> Item<S, Free> for Agg<O> {
    type Out = O;
    fn item(&self) -> pb::QuerySelect {
        select(query_select::Kind::Aggregate(self.wire.clone()))
    }
    fn read(v: &Value, path: &str) -> Result<Self::Out> {
        value(v, path)
    }
}

impl<S> Item<S, Free> for LabelOf {
    type Out = Option<Label>;
    fn item(&self) -> pb::QuerySelect {
        select(query_select::Kind::Label(self.0.clone()))
    }
    fn read(v: &Value, _: &str) -> Result<Self::Out> {
        Label::decode(v)
    }
}

/// What `select` takes: an item, or a tuple of items; `Out` is a result row.
pub trait Selection<S, I> {
    type Out;
    #[doc(hidden)]
    fn items(&self) -> Vec<pb::QuerySelect>;
    #[doc(hidden)]
    fn read(row: &[Value]) -> Result<Self::Out>;
}

/// The index of a single item.
pub struct One<I>(PhantomData<fn() -> I>);

fn cell(row: &[Value], i: usize) -> Result<&Value> {
    row.get(i)
        .ok_or_else(|| Error::internal("the platform sent a row with fewer values than the select"))
}

impl<S, I, A: Item<S, I>> Selection<S, One<I>> for A {
    type Out = A::Out;
    fn items(&self) -> Vec<pb::QuerySelect> {
        vec![self.item()]
    }
    fn read(row: &[Value]) -> Result<Self::Out> {
        A::read(cell(row, 0)?, "0")
    }
}

macro_rules! selection_tuple {
    ($(($n:ident, $i:ident, $k:tt)),+) => {
        impl<S, $($n, $i),+> Selection<S, ($($i,)+)> for ($($n,)+)
        where
            $($n: Item<S, $i>),+
        {
            type Out = ($($n::Out,)+);
            fn items(&self) -> Vec<pb::QuerySelect> {
                vec![$(self.$k.item()),+]
            }
            fn read(row: &[Value]) -> Result<Self::Out> {
                Ok(($($n::read(cell(row, $k)?, stringify!($k))?,)+))
            }
        }
    };
}

selection_tuple!((A, IA, 0));
selection_tuple!((A, IA, 0), (B, IB, 1));
selection_tuple!((A, IA, 0), (B, IB, 1), (C, IC, 2));
selection_tuple!((A, IA, 0), (B, IB, 1), (C, IC, 2), (D, ID, 3));
selection_tuple!((A, IA, 0), (B, IB, 1), (C, IC, 2), (D, ID, 3), (E, IE, 4));
selection_tuple!(
    (A, IA, 0),
    (B, IB, 1),
    (C, IC, 2),
    (D, ID, 3),
    (E, IE, 4),
    (F, IF, 5)
);
selection_tuple!(
    (A, IA, 0),
    (B, IB, 1),
    (C, IC, 2),
    (D, ID, 3),
    (E, IE, 4),
    (F, IF, 5),
    (G, IG, 6)
);
selection_tuple!(
    (A, IA, 0),
    (B, IB, 1),
    (C, IC, 2),
    (D, ID, 3),
    (E, IE, 4),
    (F, IF, 5),
    (G, IG, 6),
    (H, IH, 7)
);
selection_tuple!(
    (A, IA, 0),
    (B, IB, 1),
    (C, IC, 2),
    (D, ID, 3),
    (E, IE, 4),
    (F, IF, 5),
    (G, IG, 6),
    (H, IH, 7),
    (J, IJ, 8)
);
selection_tuple!(
    (A, IA, 0),
    (B, IB, 1),
    (C, IC, 2),
    (D, ID, 3),
    (E, IE, 4),
    (F, IF, 5),
    (G, IG, 6),
    (H, IH, 7),
    (J, IJ, 8),
    (K, IK, 9)
);
selection_tuple!(
    (A, IA, 0),
    (B, IB, 1),
    (C, IC, 2),
    (D, ID, 3),
    (E, IE, 4),
    (F, IF, 5),
    (G, IG, 6),
    (H, IH, 7),
    (J, IJ, 8),
    (K, IK, 9),
    (L, IL, 10)
);
selection_tuple!(
    (A, IA, 0),
    (B, IB, 1),
    (C, IC, 2),
    (D, ID, 3),
    (E, IE, 4),
    (F, IF, 5),
    (G, IG, 6),
    (H, IH, 7),
    (J, IJ, 8),
    (K, IK, 9),
    (L, IL, 10),
    (M, IM, 11)
);
