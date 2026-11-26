//! The plugin descriptor (`descriptor.json`, schema 1): what a plugin built with a Celobus SDK prints for
//! `--describe`. This crate holds its schema ([`Descriptor`]), its canonical JSON ([`canonical_json`]: object keys
//! sorted, arrays as declared, compact), its validation ([`validate`]: every problem collected as an [`Error`] with
//! an `E_*` code and a path such as `endpoints[PlaceOrder].roles[0]`) and the platform manifest derived from it
//! ([`to_manifest`]). The CLI runs it in `celobus plugin check`; the platform runs it again at publish.

pub mod access;
pub mod agent_files;
pub mod cron;
pub mod history;
pub mod lake;
pub mod lake_model;
pub mod load;
pub mod lock;
pub mod proto;
pub mod settings;
pub mod ui;
pub mod ui_check;
mod validate;
pub mod view;
pub mod viz_kinds;

use celobus_manifest::Manifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use cron::Cron;
pub use lock::{Bump, Change, Lock, LockedDependency, LockedEntity};
pub use proto::to_proto;
pub use validate::validate;

/// The descriptor schema this crate reads.
pub const SCHEMA: u32 = 1;

/// The capabilities a plugin may ask for (a reason is required for each).
pub const CAPABILITIES: &[&str] = &[
    "authz:bypass",
    "identity:manage",
    "push:send",
    "sql:write",
    "sql:unsafe",
    "blobs:use",
    "mcp:tools",
];

/// The engines an agent may run on; a run without one runs on the first.
pub const AGENT_ENGINES: &[&str] = &["claude", "codex", "pi"];

/// The scopes every plugin has; [`to_manifest`] adds them when the descriptor does not declare them.
pub const IMPLICIT_SCOPES: &[&str] = &["read", "write"];

/// The scope an endpoint needs when it names none.
pub const DEFAULT_SCOPE: &str = "write";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub schema: u32,
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub roles: Vec<Role>,
    #[serde(default)]
    pub scopes: Vec<Scope>,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub agents: Vec<Agent>,
    /// The connections the plugin declares: see `celobus_manifest::Connection`. Written only
    /// when declared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<Connection>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub enums: Vec<Enum>,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub entities: Vec<Entity>,
    #[serde(default)]
    pub endpoints: Vec<Endpoint>,
    /// Channels the plugin announces changes on: invalidations only, never data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topics: Vec<Topic>,
    /// The UI document: contributions, screens and overlays (`ui.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<ui::Ui>,
    /// The kinds of notification the plugin sends; sending needs the capability
    /// `push:send`. Written only when declared; not in the manifest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notifications: Vec<NotificationKind>,
    /// What the plugin asks of the agent runs it sees. Written only when
    /// declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs: Option<Runs>,
    /// The plugin's background jobs, each under its key (`remind_unpaid`). Written only when declared; not in the
    /// manifest: a job grants nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jobs: Vec<Job>,
    /// The plugin's analytic views ([`view`]): named queries over its entities, created in both engines. Written only
    /// when declared; not in the manifest: a view grants nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<view::View>,
    /// The plugin's layers of the workspace's Data Lake ([`lake`]). Written only when declared; not in the manifest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<lake::Layer>,
    /// The plugin's lake tables ([`lake`]). Written only when declared; not in the manifest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lake_tables: Vec<lake::LakeTable>,
    /// How the platform computes some of the plugin's lake tables ([`lake_model`]), when the plugin asks. Written only
    /// when declared; not in the manifest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lake_models: Vec<lake_model::LakeModel>,
    /// How the platform fills the plugin's entities kept in the analytic copy only ([`load`]), from the Data Lake,
    /// when the plugin asks for a snapshot. Written only when declared; not in the manifest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loads: Vec<load::Load>,
    /// The plugin's settings ([`settings`]): the message whose fields they are, whether the plugin checks a change
    /// (`on_change`), and the form owners and admins fill. Written only when declared; not in the manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<settings::Settings>,
}

/// The most retries a job declares, and the default.
pub const JOB_MAX_RETRIES: u32 = 20;
pub const JOB_DEFAULT_RETRIES: u32 = 3;
/// The longest delay before a retry (one hour), and the default (30 s).
pub const JOB_MAX_DELAY_MS: u64 = 3_600_000;
pub const JOB_DEFAULT_DELAY_MS: u64 = 30_000;
/// A run's timeout: from one second to one hour, five minutes by default.
pub const JOB_TIMEOUT_MS: std::ops::RangeInclusive<u64> = 1_000..=3_600_000;
pub const JOB_DEFAULT_TIMEOUT_MS: u64 = 300_000;
/// How many runs of a job run at once: 1 to 64, one by default.
pub const JOB_CONCURRENCY: std::ops::RangeInclusive<u32> = 1..=64;
pub const JOB_DEFAULT_CONCURRENCY: u32 = 1;

/// A background job: a payload message and a handler, with its retries, timeout, concurrency, schedule and who it
/// runs as. Its full name in logs and on screens is `<plugin>.<name>`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Job {
    /// The job's key in the plugin: snake_case, at most 32 characters (`remind_unpaid`).
    pub name: String,
    /// The payload message (a declared message, `RemindUnpaid`); no two jobs share one.
    pub payload: String,
    /// What the job does, for the admin's list of jobs: a key of the UI's text catalog
    /// (`job.remind_unpaid.description`) or plain text.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// A schedule ([`Cron`], five fields, UTC): each occurrence starts a run with the payload's defaults. Empty: the
    /// job runs only when enqueued.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cron: String,
    /// Runs always as the plugin's system principal, whoever enqueued them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub as_system: bool,
    /// Unset: [`JOB_DEFAULT_RETRIES`] retries [`JOB_DEFAULT_DELAY_MS`] apart, backing off exponentially.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<JobRetry>,
    /// Unset: [`JOB_DEFAULT_TIMEOUT_MS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Unset: [`JOB_DEFAULT_CONCURRENCY`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<u32>,
}

/// How a job's failed attempts are retried.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JobRetry {
    /// Attempts after the first, at most [`JOB_MAX_RETRIES`] (0: never retried).
    #[serde(default = "default_retries")]
    pub retries: u32,
    /// The delay before the first retry, at most [`JOB_MAX_DELAY_MS`].
    #[serde(default = "default_delay_ms")]
    pub delay_ms: u64,
    #[serde(default)]
    pub backoff: Backoff,
}

impl Default for JobRetry {
    fn default() -> Self {
        Self {
            retries: JOB_DEFAULT_RETRIES,
            delay_ms: JOB_DEFAULT_DELAY_MS,
            backoff: Backoff::default(),
        }
    }
}

fn default_retries() -> u32 {
    JOB_DEFAULT_RETRIES
}

fn default_delay_ms() -> u64 {
    JOB_DEFAULT_DELAY_MS
}

/// `fixed`: every retry waits the delay; `exponential`: the delay doubles with each attempt (capped at one hour).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Backoff {
    Fixed,
    #[default]
    Exponential,
}

impl Job {
    /// The retry policy, defaults filled in.
    pub fn retry_or_default(&self) -> JobRetry {
        self.retry.unwrap_or_default()
    }

    /// The timeout of a run, in milliseconds.
    pub fn timeout_ms_or_default(&self) -> u64 {
        self.timeout_ms.unwrap_or(JOB_DEFAULT_TIMEOUT_MS)
    }

    /// How many runs of the job run at once.
    pub fn concurrency_or_default(&self) -> u32 {
        self.concurrency.unwrap_or(JOB_DEFAULT_CONCURRENCY)
    }

    /// The parsed schedule: `None` for a job without one, an error for a cron that does not parse.
    pub fn schedule(&self) -> Option<Result<Cron, String>> {
        (!self.cron.is_empty()).then(|| Cron::parse(&self.cron))
    }
}

/// What a plugin asks of the agent runs it sees: the runs of its own agents and the runs its code started.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Runs {
    /// The plugin handles the run-change hook: the platform sends it a system request ([`RUN_CHANGE_METHOD`]) when
    /// such a run changes.
    #[serde(default)]
    pub on_change: bool,
}

/// The method of the run-change hook: a system request the platform sends to a plugin whose descriptor says
/// `runs.on_change`. No endpoint can be named so (a leading `_`), and the
/// platform refuses it from anyone else.
pub const RUN_CHANGE_METHOD: &str = "_RunChange";

/// The method path of a plugin's run-change hook.
pub fn run_change_path(id: &str) -> String {
    method_path(id, RUN_CHANGE_METHOD)
}

/// The method of the settings check hook: a system request the platform sends to a plugin whose descriptor says
/// `settings.on_change`, before it saves a change of the settings ([`settings`]). Reserved like
/// [`RUN_CHANGE_METHOD`].
pub const SETTINGS_CHECK_METHOD: &str = "_SettingsCheck";

/// The method path of a plugin's settings check hook.
pub fn settings_check_path(id: &str) -> String {
    method_path(id, SETTINGS_CHECK_METHOD)
}

/// A method path whose name the platform keeps for its own requests (a leading `_`).
pub fn is_reserved_method(path: &str) -> bool {
    path.rsplit_once('/')
        .is_some_and(|(_, name)| name.starts_with('_'))
}

/// The most notification kinds a plugin declares.
pub const MAX_NOTIFICATION_KINDS: usize = 20;

/// Kind names the platform keeps for its own kinds: asks (`approval`, `decision`) and agent runs (`run_*`).
pub const RESERVED_NOTIFICATION_KINDS: &[&str] = &["approval", "decision"];
pub const RESERVED_NOTIFICATION_PREFIX: &str = "run_";

/// A kind of notification: what a member turns on or off in their settings, and the inbox tab it arrives in.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NotificationKind {
    /// snake_case, at most 32 characters, unique in the plugin (`mention`)
    pub name: String,
    /// 1 to 40 characters: the settings row ("Mentions")
    pub title: String,
    /// at most 120 characters: the settings row's second line
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tab: NotificationTab,
    /// whether the daily digest includes it unless the member says otherwise
    #[serde(default)]
    pub digest: bool,
}

/// The inbox tab a kind arrives in: addressed to the member, or about something they follow.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotificationTab {
    #[default]
    ForYou,
    Watching,
}

impl NotificationTab {
    pub fn as_str(self) -> &'static str {
        match self {
            NotificationTab::ForYou => "for_you",
            NotificationTab::Watching => "watching",
        }
    }
}

/// A topic: a name, its key message, who may follow it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Topic {
    /// snake_case (`orders`)
    pub name: String,
    /// The key message (an empty message for an unkeyed topic).
    pub key: String,
    #[serde(default)]
    pub description: String,
    /// Any of these roles may follow it (none: any member who passes the scopes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// A read `ui` endpoint whose request is the key message: a keyed subscription is admitted when it answers 2xx
    /// for the key as the member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorize: Option<String>,
    /// Invalidated by plugin code (`ctx.invalidate`) rather than declared on endpoints.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub manual: bool,
}

/// An endpoint's declared invalidation: after a 2xx answer the platform invalidates the topic, keyed from the response.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Invalidation {
    pub topic: String,
    /// Key fields taken from the response (`field` of the key message ← `from` of the response); none: every key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key: Vec<KeyFrom>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct KeyFrom {
    pub field: String,
    pub from: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Role {
    pub name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub default_for: DefaultFor,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DefaultFor {
    #[default]
    None,
    Member,
    Admin,
}

impl DefaultFor {
    pub fn as_str(self) -> &'static str {
        match self {
            DefaultFor::None => "none",
            DefaultFor::Member => "member",
            DefaultFor::Admin => "admin",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub sensitive: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub name: String,
    #[serde(default)]
    pub reason: String,
    /// The scope of `authz:bypass`: the entities and operations a system context (`as_system`) may bypass the
    /// access rules of. Written only when declared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on: Vec<BypassScope>,
}

/// An entity and the operations (`read`, `create`, `update`, `delete`) of a bypass.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BypassScope {
    pub entity: String,
    pub ops: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub name: String,
    #[serde(default)]
    pub roles: Vec<String>,
    /// The workspace permissions the agent asks for (`plugins.install`): see `celobus_manifest::Agent`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace: Vec<String>,
    /// The agents this one may bring into its group: see `celobus_manifest::Agent`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delegates: Vec<String>,
    /// The `llm` connection its runs use, `<name>` or `<dependency>.<name>`: see `celobus_manifest::Agent`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub connection: String,
    /// The agent has an SSH key of its own: see `celobus_manifest::Agent`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ssh_key: bool,
    /// The agent's service account asks for base role admin: see `celobus_manifest::Agent`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub admin: bool,
    // What follows describes how the agent runs. None of it is in
    // the manifest: it grants nothing. Each member is written only when declared.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The agent's own glyph, an icon name (lucide: `rocket`, `git-pull-request`), drawn over its engine's mark
    /// wherever the agent is shown; empty: none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    /// One of [`AGENT_ENGINES`]; empty: the one its connection's protocol picks.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub engine: String,
    /// The model of its runs, when its connection lists it; empty: the connection's default model.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// Runs of this agent are sessions unless whoever starts one says otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub session: bool,
    /// The longest a run may take, in seconds, unless whoever starts one says otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_s: Option<u32>,
    /// The shared skills of the package (`skills/<name>/`) the agent has besides its own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    /// A topic of the plugin the platform invalidates, for every key, whenever a run of the agent changes: screens
    /// that follow it re-read the run through the plugin's endpoints. Empty: none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub topic: String,
}

/// A connection the plugin declares: a language model provider (`llm`), speech recognition (`stt`) or a basemap
/// provider for its maps (`maps`) the workspace's owner or admin fills in.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    /// snake_case, at most 32 characters, unique in the plugin
    pub name: String,
    /// `llm` | `stt` | `maps`
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub plugin: String,
    /// The dependency's endpoints this plugin calls.
    #[serde(default)]
    pub calls: Vec<String>,
    /// The dependency's entities this plugin reads (`ctx.query()`, declared SQL), with the fields it reads as the
    /// dependency's contract had them when `celobus plugin add` generated its client. Written only when declared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<DependencyRead>,
    /// The dependency's exported lake tables this plugin reads (`ctx.lake().query()`), and its entities whose exported
    /// histories it reads (`ctx.lake().history()`), by their names there. A plugin never names a lake table like one of
    /// its entities (`E_LAKE_TABLE_CLASH`), so each name is one of the two. Written only when declared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lake: Vec<String>,
}

/// An entity of a dependency this plugin reads; the provider must export its `read`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DependencyRead {
    /// The provider's entity (snake_case).
    pub entity: String,
    /// Its fields as read: `id` and the columns, each with its wire type (`string`, `optional int64`, ...).
    pub fields: Vec<ReadField>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReadField {
    pub name: String,
    /// The wire type as the lock writes it (`optional string`, `repeated ref:catalog.product`).
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Enum {
    pub name: String,
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    #[default]
    Message,
    Event,
    Job,
    Settings,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub name: String,
    #[serde(default)]
    pub kind: MessageKind,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub fields: Vec<Field>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    /// snake_case (`order_line`)
    pub name: String,
    /// The protobuf message of a row (`OrderLine`); an endpoint may name it as its input or output.
    pub message: String,
    /// `uuid` | `i64` | `string`
    #[serde(default = "default_key")]
    pub key: String,
    /// The field that names a row in refs and headers.
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub soft_delete: bool,
    #[serde(default)]
    pub version: bool,
    #[serde(default)]
    pub append_only: bool,
    #[serde(default)]
    pub fields: Vec<Field>,
    /// Indexes over several columns (one column: the field's `index`); written only when declared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub indexes: Vec<Index>,
    /// Who may read, create, change and delete its rows ([`access`]); none: no one but a system context.
    /// Written only when declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<access::Access>,
    /// The entity's history in the Data Lake ([`history`]): its layer, who reads it; its kept columns carry
    /// `history`. Written only when declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<history::EntityHistory>,
    /// Where its rows are kept: empty, the workspace's database (and the analytic copy, as every entity); `olap`, the
    /// analytic copy only, filled by the entity's load from the Data Lake when the plugin asks for a snapshot
    /// ([`load`]). Written only when declared.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub storage: String,
    /// With `storage` `olap`: the fields that key a row (`key(day, store, metric)`), whose values are unique among the
    /// rows a load gives; the row's `id` is made from them. Written only when declared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub olap_key: Vec<String>,
}

/// The `storage` of an entity whose rows are kept in the analytic copy only.
pub const STORAGE_OLAP: &str = "olap";

/// An index of an entity's table: its columns in order (fields or system columns), optionally unique.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub columns: Vec<IndexColumn>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unique: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct IndexColumn {
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub desc: bool,
}

/// The columns the store adds to every entity's table; a field may not take their names.
pub const SYSTEM_COLUMNS: &[&str] = &["created_at", "updated_at", "deleted_at", "version"];

/// The type an entity's `id` field has for its key (besides `ref:<plugin>.<entity>` to itself, what the SDKs print).
pub fn key_field_type(key: &str) -> Option<&'static str> {
    match key {
        "uuid" => Some("uuid"),
        "i64" => Some("int64"),
        "string" => Some("string"),
        _ => None,
    }
}

impl Entity {
    /// Whether its rows are kept in the analytic copy only (`storage` `olap`).
    pub fn olap(&self) -> bool {
        self.storage == STORAGE_OLAP
    }

    /// The system columns this entity's table has: `created_at`, `updated_at`, and `deleted_at` / `version` when
    /// declared.
    pub fn system_columns(&self) -> Vec<&'static str> {
        let mut out = vec!["created_at", "updated_at"];
        if self.soft_delete {
            out.push("deleted_at");
        }
        if self.version {
            out.push("version");
        }
        out
    }
}

fn default_key() -> String {
    "uuid".into()
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub name: String,
    pub number: u32,
    /// `string`, `bool`, `int32`, `int64`, `double`, `decimal`, `uuid`, `timestamp`, `date`, `bytes`, `json`, `file`,
    /// `secret` (settings only), `message:<Name>`, `enum:<Name>`, `ref:<plugin>.<entity>` (see [`FieldType`]).
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub repeated: bool,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub description: String,
    /// An entity column whose values are unique (among the rows that are not deleted, on a soft-delete entity).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unique: bool,
    /// An entity column with an index of its own (refs have one without it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub index: bool,
    /// An entity column the platform keeps encrypted with the workspace's key: never
    /// unique, indexed, filtered, sorted, a ref, the id, the label or read by declared SQL.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub encrypted: bool,
    /// A `string` entity column searched by full text (`search`), in this text search language ([`SEARCH_LANGUAGES`];
    /// `simple` when declared without one); empty: not searchable.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub searchable: String,
    /// A ref column's forward navigation (`include`); empty: the default, [`Field::nav_name`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub nav: String,
    /// A ref column's navigation on its target (an entity of the same plugin): the rows that point at a row of it;
    /// empty: none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub inverse: String,
    /// What a single ref column does when the row it names is deleted for good ([`ON_DELETE_OWN`] for a ref to an
    /// entity of the same plugin, [`ON_DELETE_CROSS`] for one to another plugin's or the directory's); empty: the
    /// default, [`Field::on_delete_or_default`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub on_delete: String,
    /// An entity column whose changes its history keeps ([`history`]); written only when set.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub history: bool,
    /// A `file` field's largest file, in bytes (at most [`MAX_FILE_SIZE`]); unset: the workspace's limit. Written only
    /// when declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_size: Option<u64>,
    /// A `file` field's accepted types: MIME types (`text/csv`) or patterns (`image/*`), lowercase; empty: any type.
    /// Written only when declared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accept: Vec<String>,
    // What follows belongs to the fields of the settings message only ([`settings`]; elsewhere
    // `E_FIELD_VALIDATOR_SCOPE`). Each is written only when declared.
    /// The form's label of the field (absent: the name, humanized).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The value it has until someone sets one, as the settings API writes values (numbers, decimals as strings,
    /// dates as `YYYY-MM-DD`, an enum's value name, a list as an array).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// The form's section the field is in (absent: the first, untitled one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// A string drawn as a text area.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multiline: bool,
    /// The smallest and the largest value of a number, a decimal (a string) or a date (`YYYY-MM-DD`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Value>,
    /// The shortest and the longest string or secret, in characters (Unicode code points).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_len: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_len: Option<u32>,
    /// A regular expression a string matches somewhere (anchor it with `^` and `$` to match it whole).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// `url` or `email`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// The fewest and the most items of a list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_items: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
    /// The plugin's own text for a value its validators refuse (instead of the platform's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// The type of a settings field holding a secret (an API key, a token): a string the plugin reads, kept encrypted and
/// never shown again once saved.
pub const SECRET_TYPE: &str = "secret";

/// The type of a field holding files: `File` (`{"id", "name", "type", "size"}` in JSON), a list of them when repeated.
pub const FILE_TYPE: &str = "file";

/// The largest `max_size` a file field declares: 5 TiB, the largest object an S3-compatible store keeps.
pub const MAX_FILE_SIZE: u64 = 5 << 40;

/// The most patterns a file field's `accept` lists.
pub const MAX_ACCEPT: usize = 32;

/// Reads a size as the SDKs take it: bytes (`1048576`), or a number and a unit, binary (`KiB`, `MiB`, `GiB`, `TiB`:
/// powers of 1024) or decimal (`B`, `KB`, `MB`, `GB`, `TB`: powers of 1000), a space between them optional, the unit's
/// case ignored (`1 GiB`, `500MB`, `1.5 GiB`). A fraction of a byte is dropped. `None` when it is not a size.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, unit) = (&s[..split], s[split..].trim());
    if num.is_empty() || num.starts_with('.') || num.ends_with('.') || num.matches('.').count() > 1
    {
        return None;
    }
    let mult: u64 = match unit.to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "kb" => 1_000,
        "mb" => 1_000_000,
        "gb" => 1_000_000_000,
        "tb" => 1_000_000_000_000,
        "kib" => 1 << 10,
        "mib" => 1 << 20,
        "gib" => 1 << 30,
        "tib" => 1 << 40,
        _ => return None,
    };
    let (whole, frac) = num.split_once('.').unwrap_or((num, ""));
    let whole: u64 = whole.parse().ok()?;
    let mut bytes = whole.checked_mul(mult)?;
    if !frac.is_empty() {
        let digits = frac.len().min(18) as u32;
        let frac: u64 = frac[..digits as usize].parse().ok()?;
        let scale = 10u128.pow(digits);
        let part = (u128::from(frac) * u128::from(mult)) / scale;
        bytes = bytes.checked_add(u64::try_from(part).ok()?)?;
    }
    Some(bytes)
}

/// Whether `s` is an `accept` pattern: `type/subtype` or `type/*`, lowercase MIME tokens.
pub fn accept_pattern_ok(s: &str) -> bool {
    let token = |t: &str| {
        !t.is_empty()
            && t.len() <= 127
            && t.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"!#$&^_.+-".contains(&b))
    };
    match s.split_once('/') {
        Some((ty, sub)) => token(ty) && (sub == "*" || token(sub)),
        None => false,
    }
}

/// Whether a file's type matches one of the `accept` patterns (`image/*` matches `image/png`); an empty list accepts
/// every type.
pub fn accepts(accept: &[String], content_type: &str) -> bool {
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    accept.is_empty()
        || accept.iter().any(|p| match p.strip_suffix("/*") {
            Some(ty) => ct.split_once('/').is_some_and(|(t, _)| t == ty),
            None => *p == ct,
        })
}

/// `on_delete` of a ref to an entity of the same plugin: `restrict` (the default: the target is not deleted while
/// rows refer to it), `cascade` (they are deleted with it), `set_null` (an optional ref is cleared), `no_action`
/// (as `restrict`, checked at the end of the statement).
pub const ON_DELETE_OWN: &[&str] = &["restrict", "cascade", "set_null", "no_action"];

/// `on_delete` of a ref to another plugin's entity or to the directory: `keep` (the default: the id stays) or, for a
/// ref to the directory (`platform.user`, `platform.group`), `set_null`.
pub const ON_DELETE_CROSS: &[&str] = &["keep", "set_null"];

/// The most searchable columns of an entity.
pub const MAX_SEARCHABLE: usize = 8;

/// The languages of a `searchable` column: PostgreSQL's text search configurations.
pub const SEARCH_LANGUAGES: &[&str] = &[
    "simple",
    "arabic",
    "armenian",
    "basque",
    "catalan",
    "danish",
    "dutch",
    "english",
    "estonian",
    "finnish",
    "french",
    "german",
    "greek",
    "hindi",
    "hungarian",
    "indonesian",
    "irish",
    "italian",
    "lithuanian",
    "nepali",
    "norwegian",
    "portuguese",
    "romanian",
    "russian",
    "serbian",
    "spanish",
    "swedish",
    "tamil",
    "turkish",
    "yiddish",
];

impl Field {
    /// Whether it holds files (`file`, single or a list).
    pub fn is_file(&self) -> bool {
        self.ty == FILE_TYPE
    }

    /// The forward navigation of a single ref column: its declared `nav`, else its name without `_id`
    /// (`customer_id` → `customer`; `created_by` stays `created_by`). `None` for any other field.
    pub fn nav_name(&self) -> Option<String> {
        if self.repeated || !matches!(FieldType::parse(&self.ty), Some(FieldType::Ref { .. })) {
            return None;
        }
        if !self.nav.is_empty() {
            return Some(self.nav.clone());
        }
        Some(
            self.name
                .strip_suffix("_id")
                .filter(|s| !s.is_empty())
                .unwrap_or(&self.name)
                .to_string(),
        )
    }

    /// The `on_delete` of a single ref column of plugin `plugin`'s entity: the declared one, else `restrict` for a
    /// ref to the plugin's own entity and `keep` for any other. `None` for any other field.
    pub fn on_delete_or_default(&self, plugin: &str) -> Option<String> {
        if self.repeated || self.name == "id" {
            return None;
        }
        let Some(FieldType::Ref { plugin: target, .. }) = FieldType::parse(&self.ty) else {
            return None;
        };
        if !self.on_delete.is_empty() {
            return Some(self.on_delete.clone());
        }
        Some(if target == plugin { "restrict" } else { "keep" }.to_string())
    }
}

/// A navigation of an entity, as `include` names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nav {
    /// the row the entity's ref `column` names, of `plugin`'s `entity`
    Forward {
        column: String,
        plugin: String,
        entity: String,
    },
    /// the rows of the same plugin's `entity` whose ref `column` names this row
    Inverse { entity: String, column: String },
}

/// The navigations of the plugin's entity `entity`, by name: the forward navs of its single ref columns (under
/// their nav name and under their column's name), and the inverse navs other entities (or itself) declare onto it.
pub fn navs(d: &Descriptor, entity: &str) -> Vec<(String, Nav)> {
    let mut out = vec![];
    if let Some(e) = d.entities.iter().find(|e| e.name == entity) {
        for f in e.fields.iter().filter(|f| f.name != "id") {
            let (Some(nav), Some(FieldType::Ref { plugin, entity })) =
                (f.nav_name(), FieldType::parse(&f.ty))
            else {
                continue;
            };
            let forward = Nav::Forward {
                column: f.name.clone(),
                plugin,
                entity,
            };
            if nav != f.name {
                out.push((f.name.clone(), forward.clone()));
            }
            out.push((nav, forward));
        }
    }
    for src in &d.entities {
        for f in src.fields.iter().filter(|f| !f.inverse.is_empty()) {
            if FieldType::parse(&f.ty)
                == Some(FieldType::Ref {
                    plugin: d.id.clone(),
                    entity: entity.to_string(),
                })
            {
                out.push((
                    f.inverse.clone(),
                    Nav::Inverse {
                        entity: src.name.clone(),
                        column: f.name.clone(),
                    },
                ));
            }
        }
    }
    out
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Auth {
    #[default]
    User,
    Service,
    Any,
    Public,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Idempotency {
    #[default]
    Off,
    Optional,
    Required,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    /// PascalCase (`PlaceOrder`); the method is `/<id>.v1.<Service>/<name>`.
    pub name: String,
    pub input: String,
    pub output: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub auth: Auth,
    /// Any of these roles (none: any member who passes the scope).
    #[serde(default)]
    pub roles: Vec<String>,
    /// The scope the caller's token must carry; empty means [`DEFAULT_SCOPE`].
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub idempotency: Idempotency,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// It only reads (`.read_only()` in the SDKs); UI queries may target only such endpoints.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
    /// Screens may call it (queries need it read-only).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ui: bool,
    /// Topics the platform invalidates after a 2xx answer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invalidates: Vec<Invalidation>,
}

/// A field's type, parsed from its string form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldType {
    Scalar(&'static str),
    Message(String),
    Enum(String),
    Ref { plugin: String, entity: String },
}

/// The scalar types of a field.
pub const SCALARS: &[&str] = &[
    "string",
    "bool",
    "int32",
    "int64",
    "double",
    "decimal",
    "uuid",
    "timestamp",
    "date",
    "bytes",
    "json",
    "file",
    "secret",
];

impl FieldType {
    /// `None` when the string is not a type at all (an unknown scalar, a missing name).
    pub fn parse(s: &str) -> Option<FieldType> {
        if let Some(s) = SCALARS.iter().find(|x| **x == s) {
            return Some(FieldType::Scalar(s));
        }
        let (kind, rest) = s.split_once(':')?;
        if rest.is_empty() {
            return None;
        }
        match kind {
            "message" => Some(FieldType::Message(rest.into())),
            "enum" => Some(FieldType::Enum(rest.into())),
            "ref" => {
                let (plugin, entity) = rest.split_once('.')?;
                (!plugin.is_empty() && !entity.is_empty()).then(|| FieldType::Ref {
                    plugin: plugin.into(),
                    entity: entity.into(),
                })
            }
            _ => None,
        }
    }
}

/// One validation problem: `error[<code>] at <path>: <message>`, and how to fix it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub code: &'static str,
    /// Where: `endpoints[PlaceOrder].roles[0]`, `messages[Order].fields[total].type`; empty for the whole document.
    pub path: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

impl Error {
    pub fn new(code: &'static str, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
            help: None,
        }
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.path.is_empty() {
            write!(f, "error[{}]: {}", self.code, self.message)?;
        } else {
            write!(f, "error[{}] at {}: {}", self.code, self.path, self.message)?;
        }
        if let Some(h) = &self.help {
            write!(f, "\n  help: {h}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

/// A descriptor that parsed and validated, with its canonical JSON.
#[derive(Clone, Debug)]
pub struct Checked {
    pub descriptor: Descriptor,
    /// What `descriptor.json` holds: the JSON as printed, canonicalized.
    pub canonical: String,
}

/// Parses the JSON into a [`Descriptor`]: an `E_SCHEMA` error when it is not JSON, has another `schema`, misses a
/// required member, has an unknown one or a value of the wrong type or enum.
pub fn parse(json: &[u8]) -> Result<Descriptor, Error> {
    let v: Value = serde_json::from_slice(json)
        .map_err(|e| Error::new("E_SCHEMA", "", format!("not JSON: {e}")))?;
    from_value(&v)
}

fn from_value(v: &Value) -> Result<Descriptor, Error> {
    let Some(obj) = v.as_object() else {
        return Err(Error::new(
            "E_SCHEMA",
            "",
            "the descriptor is not a JSON object",
        ));
    };
    match obj.get("schema").and_then(Value::as_u64) {
        Some(n) if n == u64::from(SCHEMA) => {}
        Some(n) => {
            return Err(Error::new(
                "E_SCHEMA",
                "schema",
                format!("schema {n} is not supported (this tool reads schema {SCHEMA})"),
            )
            .help("use an SDK and a CLI from the same platform release"));
        }
        None => {
            return Err(Error::new(
                "E_SCHEMA",
                "schema",
                format!("missing schema number (expected {SCHEMA})"),
            ));
        }
    }
    serde_path_to_error::deserialize(v).map_err(|e| {
        let path = e.path().to_string();
        let path = if path == "." { String::new() } else { path };
        Error::new("E_SCHEMA", path, e.into_inner().to_string())
    })
}

/// Parses, validates and canonicalizes: what `celobus plugin check` and the catalog run on `--describe`'s output.
pub fn check(json: &[u8]) -> Result<Checked, Vec<Error>> {
    let v: Value = serde_json::from_slice(json)
        .map_err(|e| vec![Error::new("E_SCHEMA", "", format!("not JSON: {e}"))])?;
    let descriptor = from_value(&v).map_err(|e| vec![e])?;
    let errors = validate(&descriptor);
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Checked {
        descriptor,
        canonical: canonical_json(&v),
    })
}

/// The canonical form of a JSON document: object keys sorted (by their UTF-8 bytes), arrays in their order, no
/// whitespace. Two descriptors are the same when their canonical forms are equal.
pub fn canonical_json(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(&mut out, v);
    out
}

/// [`canonical_json`] of raw JSON bytes.
pub fn canonicalize(json: &[u8]) -> Result<String, Error> {
    let v: Value = serde_json::from_slice(json)
        .map_err(|e| Error::new("E_SCHEMA", "", format!("not JSON: {e}")))?;
    Ok(canonical_json(&v))
}

fn write_canonical(out: &mut String, v: &Value) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                write_canonical(out, &m[k]);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(out, x);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// The service of a plugin's endpoints: its id in PascalCase (`orders` → `Orders`).
pub fn service_name(id: &str) -> String {
    let mut c = id.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// An endpoint's method path: `/<id>.v1.<Service>/<Name>`.
pub fn method_path(id: &str, endpoint: &str) -> String {
    format!("/{id}.v1.{}/{endpoint}", service_name(id))
}

/// The platform manifest of a (valid) descriptor.
pub fn to_manifest(d: &Descriptor) -> Manifest {
    let title = if d.title.is_empty() {
        d.id.as_str()
    } else {
        d.title.as_str()
    };
    let mut scopes: Vec<celobus_manifest::Scope> = vec![];
    for (name, label) in [("read", "Read"), ("write", "Change")] {
        if !d.scopes.iter().any(|s| s.name == name) {
            scopes.push(celobus_manifest::Scope {
                name: name.into(),
                title: format!("{label} {title}"),
                description: String::new(),
                sensitive: false,
            });
        }
    }
    scopes.extend(d.scopes.iter().map(|s| celobus_manifest::Scope {
        name: s.name.clone(),
        title: s.title.clone(),
        description: s.description.clone(),
        sensitive: s.sensitive,
    }));
    Manifest {
        roles: d
            .roles
            .iter()
            .map(|r| celobus_manifest::Role {
                name: r.name.clone(),
                title: r.title.clone(),
                description: r.description.clone(),
                default_for: Some(r.default_for.as_str().into()),
            })
            .collect(),
        scopes,
        agents: d
            .agents
            .iter()
            .map(|a| celobus_manifest::Agent {
                name: a.name.clone(),
                roles: a.roles.clone(),
                workspace: a.workspace.clone(),
                delegates: a.delegates.clone(),
                connection: a.connection.clone(),
                ssh_key: a.ssh_key,
                admin: a.admin,
            })
            .collect(),
        connections: d
            .connections
            .iter()
            .map(|c| celobus_manifest::Connection {
                name: c.name.clone(),
                kind: c.kind.clone(),
                title: c.title.clone(),
                description: c.description.clone(),
            })
            .collect(),
        depends: d.dependencies.iter().map(|x| x.plugin.clone()).collect(),
        calls: d
            .dependencies
            .iter()
            .flat_map(|x| x.calls.iter().map(|c| method_path(&x.plugin, c)))
            .collect(),
        permissions: d.permissions.clone(),
        requires: d
            .endpoints
            .iter()
            .map(|e| celobus_manifest::Requirement {
                method: method_path(&d.id, &e.name),
                scope: e
                    .scopes
                    .first()
                    .cloned()
                    .unwrap_or_else(|| DEFAULT_SCOPE.into()),
                any_role: e.roles.clone(),
            })
            .collect(),
    }
}

/// The problems of a plugin id alone (`E_PLUGIN_ID`, `E_PLUGIN_ID_RESERVED`), for `celobus plugin new`.
pub fn plugin_id_errors(id: &str, path: &str) -> Vec<Error> {
    validate::plugin_id(id, path)
}

#[cfg(test)]
mod history_tests;
#[cfg(test)]
mod lake_model_tests;
#[cfg(test)]
mod lake_tests;
#[cfg(test)]
mod load_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod ui_tests;
#[cfg(test)]
mod view_tests;
