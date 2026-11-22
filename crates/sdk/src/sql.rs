//! Reads beyond the repository: what a dependency's exported entity is (`ctx.query::<E>()`, in `query`) and SQL sent
//! as text (`ctx.sql`, `ctx.unsafe_sql`).
//!
//! A dependency's entity is a type `celobus plugin add` generates in `src/deps/<plugin>.rs`: a struct with
//! `#[entity(plugin = "...")]` and the fields the plugin reads, which implements [`Exported`] when its provider
//! exports `read`. The platform reads it through the provider's rules and masks, as the caller; only reads exist.
//!
//! SQL is a string with positional parameters (`$1` is the first value bound). `ctx.sql` reads the entities of
//! every installed plugin and the directory (`chat_post`, `platform_user`) as the caller, every entity under its own
//! rules, and never a catalog, another workspace or another session's query (403 `permission_denied`); `ctx.unsafe_sql`
//! reads (with the capability `sql:unsafe`) and writes (with `sql:write` too) the tables of
//! every plugin past their rules. The dialect is PostgreSQL. A bound value is never SQL: it takes the type the query
//! needs where it stands.
//!
//! ```ignore
//! let rows: Vec<NoteTally> = ctx
//!     .sql("select owner_id, count(*) as notes from notes_note group by owner_id having count(*) >= $1")
//!     .bind(2)
//!     .rows()
//!     .await?;
//! let customers = ctx.query::<deps::crm::Customer>().find(deps::crm::Customer::name.eq("Ada".into())).all().await?;
//! ```

use bytes::Bytes;
use celobus_proto as pb;
use pb::{syscall, syscall_result};
use serde::Serialize;
use serde_json::Value;

use crate::ctx::Ctx;
use crate::error::{Error, Result};
use crate::message::{Entity, Message};
/// A dependency's entity whose provider exports `read`: `ctx.query::<E>()` has the repository's reads of it (`get`,
/// `find`, `count_where`, `exists`, `aggregate`) besides the query's. Implemented by the clients `celobus plugin add`
/// generates, never by hand.
pub trait Exported: Entity + Message {}

/// What a plugin reads of a dependency's entity, as the generated client declares it: `.reads(deps::crm::PLUGIN,
/// deps::crm::READS)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Read {
    pub entity: &'static str,
    /// `(field, wire type)`: `("email", "optional string")`
    pub fields: &'static [(&'static str, &'static str)],
}

impl Read {
    pub const fn new(
        entity: &'static str,
        fields: &'static [(&'static str, &'static str)],
    ) -> Self {
        Self { entity, fields }
    }
}

/// SQL to run, with its parameters: `ctx.sql(..)` or `ctx.unsafe_sql(..)`, then `bind` each `$n` in turn, then
/// `rows` (each row a message, read from its columns by name) or `json`.
#[must_use = "a query runs when its rows are read"]
pub struct Sql<'a> {
    ctx: &'a Ctx,
    query: String,
    params: Vec<Value>,
    past_rules: bool,
}

impl Sql<'_> {
    /// The value of the next `$n`: a string, a number, a bool, `None` (null), or anything that serializes to JSON
    /// (an object or a list is its JSON text).
    pub fn bind(mut self, value: impl Serialize) -> Self {
        self.params
            .push(serde_json::to_value(value).unwrap_or(Value::Null));
        self
    }

    /// The rows as JSON objects, their columns by name; at most 1,000 (`query_rows_limit` beyond).
    pub async fn json(self) -> Result<Vec<Value>> {
        let body = Bytes::from(serde_json::to_vec(&self.params)?);
        let r = self
            .ctx
            .syscall(syscall::Call::Sql(pb::SqlRun {
                request_id: self.ctx.request().id(),
                tx: self.ctx.tx(),
                query: self.query,
                params: body,
                r#unsafe: self.past_rules,
            }))
            .await?;
        let Some(syscall_result::Result::DbRows(rows)) = r else {
            return Err(Error::internal(
                "the platform answered SQL with something else",
            ));
        };
        rows.rows
            .iter()
            .map(|b| {
                serde_json::from_slice(b).map_err(|e| {
                    Error::internal(format!("the platform sent a row that is not JSON: {e}"))
                })
            })
            .collect()
    }

    /// The rows as messages `R`, each read from its columns by name (`select owner_id, count(*) as notes ..` fills
    /// `owner_id` and `notes`).
    pub async fn rows<R: Message>(self) -> Result<Vec<R>> {
        self.json()
            .await?
            .iter()
            .map(|v| R::from_json(v, ""))
            .collect()
    }

    /// Runs it for its effect (a write of `unsafe_sql` without `returning`).
    pub async fn run(self) -> Result<()> {
        self.json().await.map(|_| ())
    }
}

impl Ctx {
    /// SQL that reads (`select`, `with`, `values`), over every installed plugin's entities
    /// (`<plugin>_<entity>`: `chat_post`) and the directory (`platform_user`, `platform_group`,
    /// `platform_group_member`, `platform_role_assignment`), as the caller: each entity's own rules and masks
    /// apply (in a system context, only the plugin's own entities' are lifted). Never a catalog (`pg_class`,
    /// `information_schema`, `pg_stat_activity`) or another workspace: a relation other than these, or a
    /// function in `from` other than a few set-returning built-ins (`generate_series`, `unnest`, the JSON element and
    /// record functions, `regexp_matches`, `regexp_split_to_table`, `string_to_table`), is 403 `permission_denied`.
    /// Inside a transaction (`tx.sql(..)`) it sees the transaction's writes. An encrypted field reads as its
    /// ciphertext: read it with `ctx.db`.
    pub fn sql(&self, query: impl Into<String>) -> Sql<'_> {
        Sql {
            ctx: self,
            query: query.into(),
            params: vec![],
            past_rules: false,
        }
    }

    /// SQL over the tables of every plugin of the workspace (`chat_post`, `crm_customer`) past their access rules,
    /// with the capability `sql:unsafe`: a read, or with the capability `sql:write` also an `insert`, `update` or
    /// `delete` (its `returning` rows come back). Never the platform's tables, the directory's (read it with
    /// `ctx.sql`) or a catalog (403). It runs in a system context of its own, not in a declared transaction, and each
    /// run is recorded in the workspace's audit; the tables keep their soft-deleted rows, and an encrypted field is
    /// its ciphertext.
    pub fn unsafe_sql(&self, query: impl Into<String>) -> Sql<'_> {
        Sql {
            ctx: self,
            query: query.into(),
            params: vec![],
            past_rules: true,
        }
    }
}
