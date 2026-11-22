//! The celobus plugin SDK for Rust.
//!
//! A plugin is plain Rust: `#[derive(Message)]` and `#[derive(Entity)]` on its types, builder calls for
//! its identity and endpoints, and `async fn(Ctx, XRequest) -> Result<Y>` handlers. `serve()` prints the
//! descriptor with `--describe`, otherwise keeps a session with the platform
//! (at `$CELOBUS_ADDR`) and serves the endpoints as `POST /<id>.v1.<Service>/<Name>`
//! with JSON bodies in the protobuf JSON mapping.
//!
//! ```no_run
//! use celobus_sdk::prelude::*;
//!
//! pub const CLERK: Role = Role::new("clerk").title("Order clerk");
//!
//! #[derive(Message, Clone, Debug)]
//! pub struct HelloRequest { pub name: String }
//! #[derive(Message, Clone, Debug)]
//! pub struct HelloReply { pub text: String }
//!
//! async fn hello(_ctx: Ctx, req: HelloRequest) -> Result<HelloReply> {
//!     Ok(HelloReply { text: format!("Hello, {}!", req.name) })
//! }
//!
//! fn main() {
//!     Plugin::new("hello", env!("CARGO_PKG_VERSION"))
//!         .title("Hello")
//!         .role(CLERK)
//!         .endpoint(Endpoint::new(hello).roles([CLERK]).describe("Greets."))
//!         .serve()
//! }
//! ```

extern crate self as celobus_sdk;

pub mod access;
pub mod agents;
pub mod ctx;
pub mod db;
#[cfg(test)]
mod db_tests;
pub mod descriptor;
pub mod endpoint;
pub mod error;
pub mod identity;
pub mod jobs;
#[cfg(test)]
mod jobs_tests;
pub mod message;
pub mod notify;
#[cfg(test)]
mod notify_tests;
pub mod platform;
pub mod plugin;
pub mod query;
pub mod runlog;
mod session;
pub mod sql;
pub mod types;
pub mod ui;

pub use agents::{
    AgentActor, AgentEvent, AgentEvents, AgentPartial, AgentPartialMessage, AgentRun,
    AgentRunError, AgentRunStatus, AvailableAgent, RunChange, RunOptions,
};
pub use celobus_sdk_derive::{Entity, Message};
pub use ctx::{Caller, CallerKind, Ctx, PluginInfo, RequestInfo};
pub use db::{
    Aggregate, Db, Filter, Find, Get, Grouped, Include, Inverse, Nav, Order, Page, Patch, Row, Tx,
    TxDb, TxOptions,
};
pub use descriptor::{DescribeError, DescribeErrors, Descriptor};
pub use endpoint::{Auth, Endpoint, Idempotency};
pub use error::{Error, ErrorCode, Result};
pub use identity::{
    Agent, AgentEngine, Capability, Connection, ConnectionKind, DefaultFor, Role, Scope,
};
pub use jobs::{Enqueued, Job, JobInfo, Jobs, Retry, RunState};
pub use message::{Call, Entity, Enum, FieldValue, Message};
pub use notify::Notifications;
pub use plugin::Plugin;
pub use query::Query;
pub use runlog::run_log_layer;
pub use sql::{Exported, Read, Sql};
pub use types::{Date, Decimal, Id, Json, Timestamp, Uuid};

/// `use celobus_sdk::prelude::*;`: everything a plugin file needs.
pub mod prelude {
    pub use crate::access::{Access, Grant, Perm, Rel, Relation};
    pub use crate::agents::{
        AgentEvent, AgentEvents, AgentRun, AgentRunStatus, AvailableAgent, RunChange, RunOptions,
    };
    pub use crate::ctx::{Caller, CallerKind, Ctx};
    pub use crate::db::{Filter, Page, Patch, Row, Tx, TxOptions};
    pub use crate::endpoint::{Auth, Endpoint, Idempotency};
    pub use crate::error::{Error, ErrorCode, Result};
    pub use crate::identity::{
        Agent, AgentEngine, Capability, Connection, ConnectionKind, DefaultFor, Role, Scope,
    };
    pub use crate::jobs::{Job, Retry};
    pub use crate::message::{Call, Entity, Enum, Message};
    pub use crate::platform;
    pub use crate::plugin::Plugin;
    pub use crate::query::ColumnCompare;
    pub use crate::sql::{Exported, Read, Sql};
    pub use crate::types::{Date, Decimal, Id, Json, Timestamp, Uuid};
    pub use celobus_sdk_derive::{Entity, Message};
}

/// Used by the code the derives generate; not a public API.
#[doc(hidden)]
pub mod __private {
    pub use crate::error::{Error, Result};
    pub use crate::message::{
        Call, Entity, EntityInfo, Enum, FieldType, FieldValue, IndexInfo, Message, MessageInfo,
        Registry, decode_enum, decode_field, encode_field, encode_field_trimmed, object,
    };
    pub use crate::query::{Nullable, Queryable};
    pub use crate::types::{Id, Uuid};
    pub use serde;
    pub use serde_json::{Map, Value};
}
