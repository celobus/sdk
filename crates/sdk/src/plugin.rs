//! `Plugin`: the declaration builder, the descriptor it makes, and `serve()`.

use std::collections::HashSet;
use std::io::Write as _;
use std::time::Duration;

use crate::descriptor::*;
use crate::endpoint::Endpoint;
use crate::identity::{Agent, Capability, Role, Scope};
use crate::message::{Message, Registry};

/// Ids the platform keeps for itself (and every id beginning with `pg`).
pub const RESERVED_IDS: &[&str] = &[
    "platform",
    "celobus",
    "core",
    "system",
    "sys",
    "workspace",
    "admin",
    "api",
    "app",
    "auth",
    "ui",
    "sdk",
    "cell",
    "public",
    "internal",
    "mcp",
];

/// Adds a message or an entity (and what it uses) to the registry.
type Register = fn(&mut Registry);

const MAX_TIMEOUT: Duration = Duration::from_secs(600);

/// A plugin: its identity and its subjects. `serve()` runs it; with `--describe` it prints the
/// descriptor and exits.
///
/// ```no_run
/// use celobus_sdk::prelude::*;
///
/// #[derive(Message, Clone, Debug)]
/// pub struct HelloRequest { pub name: String }
/// #[derive(Message, Clone, Debug)]
/// pub struct HelloReply { pub text: String }
///
/// async fn hello(_ctx: Ctx, req: HelloRequest) -> Result<HelloReply> {
///     Ok(HelloReply { text: format!("Hello, {}!", req.name) })
/// }
///
/// fn main() {
///     Plugin::new("hello", env!("CARGO_PKG_VERSION")).endpoint(Endpoint::new(hello)).serve()
/// }
/// ```
pub struct Plugin {
    pub(crate) id: String,
    pub(crate) version: String,
    title: Option<String>,
    description: String,
    roles: Vec<Role>,
    scopes: Vec<Scope>,
    capabilities: Vec<(Capability, String)>,
    permissions: Vec<String>,
    agents: Vec<Agent>,
    connections: Vec<crate::identity::Connection>,
    dependencies: Vec<(String, Vec<String>)>,
    /// what the plugin reads of its dependencies' exported entities, by provider
    reads: Vec<(String, Vec<crate::sql::Read>)>,
    notifications: Vec<crate::notify::Kind>,
    entities: Vec<(&'static str, Register)>,
    /// entity name -> its access rules
    accesses: Vec<(&'static str, serde_json::Value)>,
    /// the scope of `authz:bypass`: entity and operations
    bypass: Vec<serde_json::Value>,
    messages: Vec<Register>,
    /// the analytic views: their row message, and their declaration (or the mistake made building it), with the
    /// row message's name (whose description is the view's)
    views: Vec<(
        Register,
        &'static str,
        Result<celobus_descriptor::view::View, String>,
    )>,
    pub(crate) endpoints: Vec<Endpoint>,
    topics: Vec<crate::ui::TopicDecl>,
    ui: Option<crate::ui::doc::Ui>,
    /// the run-change hook (`on_run_change`)
    pub(crate) on_run_change: Option<crate::endpoint::Handler>,
    pub(crate) jobs: Vec<crate::jobs::JobDecl>,
    /// how many runs the process starts at once
    pub(crate) job_slots: u32,
    /// the Data Lake: the layers, the tables (their row message, the message's name), who reads them, and other
    /// plugins' tables read
    layers: Vec<crate::lake::Layer>,
    lake_tables: Vec<(Register, &'static str, LakeDecl)>,
    lake_reads: Vec<(&'static str, Vec<String>)>,
    lake_system: Vec<&'static str>,
    lake_exports: Vec<&'static str>,
    lake_model_exports: Vec<&'static str>,
    lake_deps: Vec<(&'static str, &'static str)>,
    /// the entities' histories: who reads each, which the system context reads, which are exported
    history_reads: Vec<(&'static str, Vec<String>)>,
    history_system: Vec<&'static str>,
    history_exports: Vec<&'static str>,
    /// mistakes of the lake declarations (reported by `descriptor`)
    lake_errors: Vec<String>,
    /// the models of lake tables (or the mistake made building one)
    lake_models: Vec<Result<celobus_descriptor::lake_model::LakeModel, String>>,
    /// the loads of entities stored in OLAP (or the mistake made building one)
    loads: Vec<Result<celobus_descriptor::load::Load, String>>,
    /// the settings (`settings`)
    pub(crate) settings: Option<crate::settings::SettingsDecl>,
}

/// A lake table as `lake_table::<T>()` declared it.
struct LakeDecl {
    name: &'static str,
    layer: &'static str,
    partition: &'static [&'static str],
    key: &'static [&'static str],
}

impl Plugin {
    /// `id`: `^[a-z][a-z0-9]{1,15}$`, not reserved; `version`: semver (`env!("CARGO_PKG_VERSION")`).
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            title: None,
            description: String::new(),
            roles: Vec::new(),
            scopes: Vec::new(),
            capabilities: Vec::new(),
            permissions: Vec::new(),
            agents: Vec::new(),
            connections: Vec::new(),
            dependencies: Vec::new(),
            reads: Vec::new(),
            notifications: Vec::new(),
            entities: Vec::new(),
            accesses: Vec::new(),
            bypass: Vec::new(),
            messages: Vec::new(),
            views: Vec::new(),
            endpoints: Vec::new(),
            topics: Vec::new(),
            ui: None,
            on_run_change: None,
            jobs: Vec::new(),
            job_slots: crate::jobs::DEFAULT_JOB_SLOTS,
            layers: Vec::new(),
            lake_tables: Vec::new(),
            lake_reads: Vec::new(),
            lake_system: Vec::new(),
            lake_exports: Vec::new(),
            lake_model_exports: Vec::new(),
            lake_deps: Vec::new(),
            history_reads: Vec::new(),
            history_system: Vec::new(),
            history_exports: Vec::new(),
            lake_errors: Vec::new(),
            lake_models: Vec::new(),
            loads: Vec::new(),
            settings: None,
        }
    }

    /// Default: the id with a capital first letter.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn role(mut self, role: Role) -> Self {
        self.roles.push(role);
        self
    }

    /// Declares a scope (or titles `Scope::READ` / `Scope::WRITE`, which exist without it).
    pub fn scope(mut self, scope: Scope) -> Self {
        self.scopes.push(scope);
        self
    }

    /// A capability, with the reason shown at install.
    pub fn capability(mut self, capability: Capability, reason: impl Into<String>) -> Self {
        self.capabilities.push((capability, reason.into()));
        self
    }

    /// Directory data beyond names (`members.email`).
    pub fn permission(mut self, permission: impl Into<String>) -> Self {
        self.permissions.push(permission.into());
        self
    }

    /// An agent of the plugin: `.agent(Agent::new("assistant").roles([MANAGER]).engine(AgentEngine::Claude))`.
    /// Declares a kind of notification the plugin sends (`ctx.notifications()`); needs the capability
    /// `push:send` (`.capability(Capability::PUSH_SEND, "<why>")`).
    pub fn notification(mut self, kind: crate::notify::Kind) -> Self {
        self.notifications.push(kind);
        self
    }

    pub fn agent(mut self, agent: Agent) -> Self {
        self.agents.push(agent);
        self
    }

    /// A connection of the plugin: `.connection(Connection::llm("research").title("Research
    /// model"))`. Installing the plugin creates it, not set up, in the workspace.
    pub fn connection(mut self, connection: crate::identity::Connection) -> Self {
        self.connections.push(connection);
        self
    }

    /// A plugin this one calls, with the endpoints it calls (`.depends_on("customers", ["GetCustomer"])`).
    pub fn depends_on<S: Into<String>>(
        mut self,
        plugin: impl Into<String>,
        calls: impl IntoIterator<Item = S>,
    ) -> Self {
        self.dependencies
            .push((plugin.into(), calls.into_iter().map(Into::into).collect()));
        self
    }

    /// The entities of a dependency this plugin reads (`ctx.query::<E>()`, declared SQL), as its generated client
    /// declares them: `.reads(deps::crm::PLUGIN, deps::crm::READS)`. A dependency not declared with `depends_on`
    /// is added with no calls.
    pub fn reads(
        mut self,
        plugin: impl Into<String>,
        reads: impl IntoIterator<Item = crate::sql::Read>,
    ) -> Self {
        let plugin = plugin.into();
        let reads: Vec<crate::sql::Read> = reads.into_iter().collect();
        match self.reads.iter_mut().find(|(p, _)| *p == plugin) {
            Some((_, r)) => r.extend(reads),
            None => self.reads.push((plugin, reads)),
        }
        self
    }

    /// A message no endpoint uses (an event payload, a type for dependents).
    pub fn message<M: Message>(mut self) -> Self {
        self.messages.push(M::register);
        self
    }

    /// An entity (`#[derive(Entity)]`).
    pub fn entity<E: Message + crate::message::Entity>(mut self) -> Self {
        self.entities.push((E::NAME, E::register));
        self
    }

    /// An analytic view (`crate::query::View`): a named query over the plugin's entities whose rows are `V`
    /// (`#[derive(View)]`), created by the platform in both engines; read with `ctx.query::<V>()` or
    /// `ctx.analytics().query::<V>()`.
    pub fn view<V: Message + crate::query::Queryable>(
        mut self,
        view: crate::query::View<V>,
    ) -> Self {
        self.views
            .push((V::register, <V as Message>::NAME, view.decl(String::new())));
        self
    }

    /// A layer of the Data Lake its tables belong to (`Layer::new("raw").title("As it arrived")`).
    pub fn layer(mut self, layer: crate::lake::Layer) -> Self {
        self.layers.push(layer);
        self
    }

    /// A lake table (`#[derive(LakeTable)]`), in a layer the plugin declares.
    pub fn lake_table<T: crate::lake::LakeTable>(mut self) -> Self {
        if T::OWNER.is_some() {
            self.lake_errors.push(format!(
                "{} is another plugin's lake table: declare its read with lake_dependency",
                T::TABLE
            ));
            return self;
        }
        self.lake_tables.push((
            T::register,
            <T as Message>::NAME,
            LakeDecl {
                name: T::TABLE,
                layer: T::LAYER.name(),
                partition: T::PARTITION,
                key: T::KEY,
            },
        ));
        self
    }

    /// How the platform computes a lake table of the plugin (`crate::lake::Model`), when the plugin asks
    /// (`ctx.lake().refresh::<T>()`): `.lake_table::<Stock>().model(stock())`.
    pub fn model<T: crate::lake::LakeTable>(mut self, model: crate::lake::Model<T>) -> Self {
        self.lake_models.push(model.decl());
        self
    }

    /// How the platform fills an entity stored in OLAP (`#[entity(storage = olap, key(..))]`) from the lake when the
    /// plugin asks for a snapshot that loads it (`ctx.analytics().snapshot().load::<E>().request()`):
    /// `.entity::<StoreMetric>().load(store_metric_load())`.
    pub fn load<E: crate::message::Entity + crate::query::Queryable + Message>(
        mut self,
        load: crate::lake::Load<E>,
    ) -> Self {
        self.loads.push(load.decl());
        self
    }

    /// Who reads a lake table of the plugin: the holders of these roles, every row of it (`ctx.lake().query()`).
    pub fn lake_read<T: crate::lake::LakeTable>(
        mut self,
        roles: impl IntoIterator<Item = Role>,
    ) -> Self {
        let roles: Vec<String> = roles.into_iter().map(|r| r.name().to_string()).collect();
        match self.lake_reads.iter_mut().find(|(t, _)| *t == T::TABLE) {
            Some((_, r)) => r.extend(roles),
            None => self.lake_reads.push((T::TABLE, roles)),
        }
        self
    }

    /// The plugin's own system context (`ctx.as_system(..)`) reads the lake table too.
    pub fn lake_read_as_system<T: crate::lake::LakeTable>(mut self) -> Self {
        self.lake_system.push(T::TABLE);
        self
    }

    /// Other plugins that declare it (`lake_dependency`) read the lake table, for callers holding one of its read
    /// roles.
    pub fn lake_export<T: crate::lake::LakeTable>(mut self) -> Self {
        self.lake_exports.push(T::TABLE);
        self
    }

    /// Other plugins that declare the read (`lake_dependency`) may copy the lake table into their own lake tables with a
    /// model (`Plugin::model`): there their own roles decide who reads the copy, every row of it. Apart from
    /// `lake_export`, which lets their callers read it by this table's own roles.
    pub fn lake_export_to_models<T: crate::lake::LakeTable>(mut self) -> Self {
        self.lake_model_exports.push(T::TABLE);
        self
    }

    /// Another plugin's exported lake table this plugin reads (`#[lake(plugin = "retail", name = "stock")]`).
    pub fn lake_dependency<T: crate::lake::LakeTable>(mut self) -> Self {
        match T::OWNER {
            Some(p) => self.lake_deps.push((p, T::TABLE)),
            None => self.lake_errors.push(format!(
                "{} is this plugin's own lake table: lake_dependency names another plugin's (#[lake(plugin = ..)])",
                T::TABLE
            )),
        }
        self
    }

    /// Who reads the history of an entity that keeps one (`#[entity(history(layer = ..))]`, `crate::lake::history`):
    /// the holders of these roles, every row of it (`ctx.lake().history::<E>()`).
    pub fn history_read<E: crate::message::Entity>(
        mut self,
        roles: impl IntoIterator<Item = Role>,
    ) -> Self {
        let roles: Vec<String> = roles.into_iter().map(|r| r.name().to_string()).collect();
        match self.history_reads.iter_mut().find(|(e, _)| *e == E::ENTITY) {
            Some((_, r)) => r.extend(roles),
            None => self.history_reads.push((E::ENTITY, roles)),
        }
        self
    }

    /// The plugin's own system context (`ctx.as_system(..)`) reads the entity's history too.
    pub fn history_read_as_system<E: crate::message::Entity>(mut self) -> Self {
        self.history_system.push(E::ENTITY);
        self
    }

    /// Other plugins that declare it (`history_dependency`) read the entity's history, for callers holding one of its
    /// read roles.
    pub fn history_export<E: crate::message::Entity>(mut self) -> Self {
        self.history_exports.push(E::ENTITY);
        self
    }

    /// Another plugin's exported history this plugin reads (`#[entity(plugin = "helpdesk")]` on its entity).
    pub fn history_dependency<E: crate::message::Entity>(mut self) -> Self {
        match E::PLUGIN {
            Some(p) => self.lake_deps.push((p, E::ENTITY)),
            None => self.lake_errors.push(format!(
                "{} is this plugin's own entity: history_dependency names another plugin's (#[entity(plugin = ..)])",
                E::ENTITY
            )),
        }
        self
    }

    /// The access rules of an entity (`crate::access`): without them no one but a system context reads or writes
    /// its rows.
    pub fn access<E: crate::message::Entity>(mut self, access: crate::access::Access<E>) -> Self {
        let v = serde_json::to_value(&access.decl).expect("access rules serialize");
        self.accesses.push((E::ENTITY, v));
        self
    }

    /// The scope of the `authz:bypass` capability: what a system context (`ctx.as_system(reason)`) may do on an
    /// entity, with `Perm::READ`, `CREATE`, `UPDATE`, `DELETE`.
    pub fn bypass<E: crate::message::Entity>(
        mut self,
        ops: impl IntoIterator<Item = crate::access::Perm>,
    ) -> Self {
        let ops: Vec<crate::access::Perm> = ops.into_iter().collect();
        self.bypass
            .push(crate::access::bypass_decl(E::ENTITY, &ops));
        self
    }

    pub fn endpoint(mut self, endpoint: Endpoint) -> Self {
        self.endpoints.push(endpoint);
        self
    }

    /// A topic screens follow (`ORDERS.roles([CLERK])`); endpoints declare that they invalidate it.
    pub fn topic(mut self, t: impl Into<crate::ui::TopicDecl>) -> Self {
        self.topics.push(t.into());
        self
    }

    /// The plugin's UI: sidebar, screens, overlays, commands, home widgets (`crate::ui`).
    pub fn ui(mut self, ui: crate::ui::Ui) -> Self {
        self.ui = Some(ui.finish());
        self
    }

    /// The run-change hook: the platform calls `handler` when a run the plugin
    /// sees changes — a run of one of its agents, or one its code started — with where the run stands
    /// ([`crate::RunChange`]). A system request: no caller (`ctx.caller()` is the system), so the handler writes
    /// with `ctx.as_system(..)` and reads the run with `ctx.agents()`.
    ///
    /// Coalesced per run (one call per 250 ms window, then a trailing one with the latest state) and delivered at
    /// least once: an error answer, a timeout or a restart of the plugin makes the platform call again (after 1 s, 5 s,
    /// 30 s, 30 s; then it gives up and logs it). Handle it idempotently. The platform never waits for it to report a
    /// run.
    ///
    /// ```no_run
    /// use celobus_sdk::prelude::*;
    ///
    /// async fn run_changed(ctx: Ctx, change: RunChange) -> Result<()> {
    ///     if change.ended {
    ///         let run = ctx.agents().get(&change.run_id).await?;
    ///         // store the answer, idempotently (the run id as a unique key)
    ///         let _ = run.result;
    ///     }
    ///     Ok(())
    /// }
    ///
    /// fn main() {
    ///     Plugin::new("helpdesk", env!("CARGO_PKG_VERSION")).on_run_change(run_changed).serve()
    /// }
    /// ```
    pub fn on_run_change<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(crate::ctx::Ctx, crate::agents::RunChange) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = crate::error::Result<()>> + Send + 'static,
    {
        let handler = std::sync::Arc::new(handler);
        self.on_run_change = Some(std::sync::Arc::new(
            move |ctx: crate::ctx::Ctx, body: serde_json::Value| {
                let handler = handler.clone();
                Box::pin(async move {
                    let change = crate::agents::RunChange::from_json(&body)?;
                    handler(ctx, change).await?;
                    Ok(serde_json::Value::Object(Default::default()))
                })
            },
        ));
        self
    }

    /// The plugin's settings ([`crate::settings`]): what an owner or an admin of the workspace sets for it, one
    /// `#[message(settings)]` struct read with `ctx.settings::<S>()`:
    /// `.settings(Settings::<ChatSettings>::new().on_change(check))`.
    pub fn settings<S: Message>(mut self, settings: crate::settings::Settings<S>) -> Self {
        self.settings = Some(settings.into());
        self
    }

    /// A background job (`crate::jobs`): `.job(remind_unpaid::job())`.
    pub fn job<P: Message>(mut self, job: crate::jobs::Job<P>) -> Self {
        self.jobs.push(job.decl);
        self
    }

    /// How many runs of its jobs one process of the plugin runs at once, apart from its requests: 1 to 64, 16 by
    /// default. The platform never hands a process more runs than this at once.
    pub fn job_slots(mut self, n: u32) -> Self {
        self.job_slots = n;
        self
    }

    /// The plugin's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The definition errors the SDK can see (empty when the plugin is well-formed).
    pub fn check(&self) -> Vec<DescribeError> {
        match self.descriptor() {
            Ok(_) => Vec::new(),
            Err(e) => e.0,
        }
    }

    /// The canonical descriptor JSON (`--describe`), or every definition error.
    pub fn describe_json(&self) -> Result<String, DescribeErrors> {
        self.descriptor().map(|d| d.to_canonical_json())
    }

    /// The descriptor, or every definition error.
    pub fn descriptor(&self) -> Result<Descriptor, DescribeErrors> {
        let mut errors = Vec::new();
        let err = |errors: &mut Vec<DescribeError>, code, path: String, message: String| {
            errors.push(DescribeError::new(code, path, message))
        };

        if !plugin_id_ok(&self.id) {
            err(
                &mut errors,
                "E_PLUGIN_ID",
                "id".into(),
                format!("`{}` must match ^[a-z][a-z0-9]{{1,15}}$", self.id),
            );
        } else if RESERVED_IDS.contains(&self.id.as_str()) || self.id.starts_with("pg") {
            err(
                &mut errors,
                "E_PLUGIN_ID_RESERVED",
                "id".into(),
                format!(
                    "`{}` is reserved (reserved: {}, pg*)",
                    self.id,
                    RESERVED_IDS.join(", ")
                ),
            );
        }
        if !semver_ok(&self.version) {
            err(
                &mut errors,
                "E_PLUGIN_VERSION",
                "version".into(),
                format!("`{}` is not a semantic version", self.version),
            );
        }

        let mut seen = HashSet::new();
        for (i, role) in self.roles.iter().enumerate() {
            if !key_ok(role.name()) {
                err(
                    &mut errors,
                    "E_ROLE_NAME",
                    format!("roles[{i}]"),
                    format!("`{}` must match ^[a-z][a-z0-9_]{{1,31}}$", role.name()),
                );
            } else if !seen.insert(role.name()) {
                err(
                    &mut errors,
                    "E_ROLE_DUP",
                    format!("roles[{i}]"),
                    format!("role `{}` is declared twice", role.name()),
                );
            }
        }
        let mut seen = HashSet::new();
        for (i, scope) in self.scopes.iter().enumerate() {
            if !key_ok(scope.name()) {
                err(
                    &mut errors,
                    "E_SCOPE_NAME",
                    format!("scopes[{i}]"),
                    format!("`{}` must match ^[a-z][a-z0-9_]{{1,31}}$", scope.name()),
                );
            } else if !seen.insert(scope.name()) {
                err(
                    &mut errors,
                    "E_SCOPE_DUP",
                    format!("scopes[{i}]"),
                    format!("scope `{}` is declared twice", scope.name()),
                );
            }
        }
        for (i, (cap, reason)) in self.capabilities.iter().enumerate() {
            if !Capability::KNOWN.contains(cap) {
                err(
                    &mut errors,
                    "E_CAPABILITY_UNKNOWN",
                    format!("capabilities[{i}]"),
                    format!("unknown capability `{}`", cap.name()),
                );
            }
            if reason.trim().is_empty() {
                err(
                    &mut errors,
                    "E_CAPABILITY_REASON",
                    format!("capabilities[{i}].reason"),
                    format!("capability `{}` needs a reason", cap.name()),
                );
            }
        }
        let mut seen = HashSet::new();
        for (i, (dep, _)) in self.dependencies.iter().enumerate() {
            if *dep == self.id {
                err(
                    &mut errors,
                    "E_DEP_SELF",
                    format!("dependencies[{i}]"),
                    "a plugin cannot depend on itself".into(),
                );
            } else if !seen.insert(dep.as_str()) {
                err(
                    &mut errors,
                    "E_DEP_DUP",
                    format!("dependencies[{i}]"),
                    format!("dependency `{dep}` is declared twice"),
                );
            }
        }

        // The walk that orders `messages` and `enums` (the same in every SDK): the endpoints as declared
        // (input, then output), then the entities as declared, then the extra messages; each message
        // before the messages and enums its fields use, depth first, in field order.
        let mut reg = Registry::default();
        for endpoint in &self.endpoints {
            (endpoint.register)(&mut reg);
        }
        for (_, register) in &self.entities {
            register(&mut reg);
        }
        for register in &self.messages {
            register(&mut reg);
        }
        // then the jobs' payloads
        for j in &self.jobs {
            (j.register)(&mut reg);
        }
        // then the topics' keys
        for t in &self.topics {
            (t.register)(&mut reg);
        }
        // then the views' rows
        for (register, _, _) in &self.views {
            register(&mut reg);
        }
        // then the lake tables' rows
        for (register, _, _) in &self.lake_tables {
            register(&mut reg);
        }
        // then the settings message
        if let Some(s) = &self.settings {
            (s.register)(&mut reg);
        }
        errors.append(&mut reg.errors);

        // `columns`: an entity's fields, with their column modifiers (its message's copy has none)
        let fields_of = |fields: &[crate::message::FieldInfo], columns: bool| -> Vec<FieldDecl> {
            fields
                .iter()
                .enumerate()
                .map(|(i, f)| FieldDecl {
                    name: f.name.to_string(),
                    number: i as u32 + 1,
                    ty: f.ty.spelled(&self.id),
                    repeated: f.ty.repeated,
                    optional: f.ty.optional,
                    description: f.description.to_string(),
                    unique: columns && f.unique,
                    index: columns && f.index,
                    encrypted: columns && f.encrypted,
                    searchable: if columns { f.searchable } else { "" }.to_string(),
                    nav: if columns { f.nav } else { "" }.to_string(),
                    inverse: if columns { f.inverse } else { "" }.to_string(),
                    on_delete: if columns { f.on_delete } else { "" }.to_string(),
                    history: columns && f.history,
                    max_size: f.max_size,
                    accept: f.accept.iter().map(|a| a.to_string()).collect(),
                    settings: f
                        .settings
                        .iter()
                        .map(|(k, v)| {
                            let v = serde_json::from_str(v).unwrap_or(serde_json::Value::Null);
                            (k.to_string(), v)
                        })
                        .collect(),
                })
                .collect()
        };
        let fields = |fields: &[crate::message::FieldInfo]| fields_of(fields, false);

        let enums: Vec<EnumDecl> = reg
            .enums
            .iter()
            .map(|e| EnumDecl {
                name: e.name.to_string(),
                values: e.values.iter().map(|v| v.to_string()).collect(),
            })
            .collect();
        // an entity's message is a message too (its fields are the entity's)
        let messages: Vec<MessageDecl> = reg
            .order
            .iter()
            .filter_map(|name| {
                if let Some(m) = reg.messages.iter().find(|m| m.name == *name) {
                    return Some(MessageDecl {
                        name: m.name.to_string(),
                        kind: m.kind.to_string(),
                        description: m.description.to_string(),
                        fields: fields(&m.fields),
                    });
                }
                let e = reg.entities.iter().find(|e| e.message == *name)?;
                Some(MessageDecl {
                    name: e.message.to_string(),
                    kind: "message".to_string(),
                    description: e.description.to_string(),
                    fields: fields(&e.fields),
                })
            })
            .collect();
        // entities as declared, then any an endpoint or a message uses without `.entity()`
        let mut ordered: Vec<&crate::message::EntityInfo> = Vec::new();
        for (message, _) in &self.entities {
            if let Some(e) = reg.entities.iter().find(|e| e.message == *message)
                && !ordered.iter().any(|o| o.message == e.message)
            {
                ordered.push(e);
            }
        }
        for name in &reg.order {
            if let Some(e) = reg.entities.iter().find(|e| e.message == *name)
                && !ordered.iter().any(|o| o.message == e.message)
            {
                ordered.push(e);
            }
        }
        let mut entities = Vec::new();
        let mut seen = HashSet::new();
        for (i, e) in ordered.into_iter().enumerate() {
            if e.name.ends_with("_with_deleted") {
                err(
                    &mut errors,
                    "E_ENT_NAME_RESERVED",
                    format!("entities[{i}]"),
                    format!("entity `{}` may not end in _with_deleted", e.name),
                );
            }
            if !seen.insert(e.name) {
                err(
                    &mut errors,
                    "E_ENT_DUP",
                    format!("entities[{i}]"),
                    format!("entity `{}` is declared twice", e.name),
                );
            }
            entities.push(EntityDecl {
                access: self
                    .accesses
                    .iter()
                    .find(|(name, _)| *name == e.name)
                    .map(|(_, a)| a.clone()),
                name: e.name.to_string(),
                message: e.message.to_string(),
                key: e.key.to_string(),
                label: e.label.to_string(),
                soft_delete: e.soft_delete,
                version: e.version,
                append_only: e.append_only,
                fields: fields_of(&e.fields, true),
                indexes: e
                    .indexes
                    .iter()
                    .map(|ix| IndexDecl {
                        columns: ix
                            .columns
                            .iter()
                            .map(|(name, desc)| IndexColumnDecl {
                                name: name.to_string(),
                                desc: *desc,
                            })
                            .collect(),
                        unique: ix.unique,
                    })
                    .collect(),
                history: e.history.as_ref().map(|h| {
                    let read = roles_once(
                        self.history_reads
                            .iter()
                            .filter(|(n, _)| *n == e.name)
                            .flat_map(|(_, r)| r.iter().cloned()),
                    );
                    celobus_descriptor::history::EntityHistory {
                        layer: h.layer.name().to_string(),
                        all: h.all,
                        unmasked: h.unmasked,
                        read,
                        system_read: self.history_system.contains(&e.name),
                        export: self.history_exports.contains(&e.name),
                    }
                }),
                storage: e.storage.to_string(),
                olap_key: e.olap_key.iter().map(|k| k.to_string()).collect(),
            });
        }
        for (what, names) in [
            (
                "history_read",
                self.history_reads
                    .iter()
                    .map(|(n, _)| *n)
                    .collect::<Vec<_>>(),
            ),
            ("history_read_as_system", self.history_system.clone()),
            ("history_export", self.history_exports.clone()),
        ] {
            for n in names {
                if !entities
                    .iter()
                    .any(|e: &EntityDecl| e.name == n && e.history.is_some())
                {
                    err(
                        &mut errors,
                        "E_HISTORY_DECLARE",
                        format!("entities[{n}].history"),
                        format!(
                            "{what}::<{n}> names an entity that keeps no history: declare #[entity(history(layer = ..))]"
                        ),
                    );
                }
            }
        }

        for (i, (name, _)) in self.accesses.iter().enumerate() {
            if !entities.iter().any(|e: &EntityDecl| e.name == *name) {
                err(
                    &mut errors,
                    "E_ACCESS_ENTITY",
                    format!("access[{i}]"),
                    format!(
                        "access rules for `{name}`, which is not an entity of the plugin (`.entity::<..>()`)"
                    ),
                );
            }
        }
        if !self.bypass.is_empty()
            && !self
                .capabilities
                .iter()
                .any(|(c, _)| *c == Capability::AUTHZ_BYPASS)
        {
            err(
                &mut errors,
                "E_CAPABILITY_SCOPE",
                "capabilities".into(),
                "a bypass scope needs the capability authz:bypass".into(),
            );
        }
        let declared_roles: HashSet<&str> = self.roles.iter().map(|r| r.name()).collect();
        let mut known_scopes: HashSet<&str> = self.scopes.iter().map(|s| s.name()).collect();
        known_scopes.extend(["read", "write", "decrypt"]);
        let mut endpoints = Vec::new();
        let mut seen = HashSet::new();
        for (i, e) in self.endpoints.iter().enumerate() {
            let path = format!("endpoints[{i}]");
            let name = match e.name() {
                Some(name) => {
                    if !seen.insert(name) {
                        err(
                            &mut errors,
                            "E_ENDPOINT_DUP",
                            path.clone(),
                            format!("endpoint `{name}` is declared twice"),
                        );
                    }
                    name
                }
                None => {
                    err(
                        &mut errors,
                        "E_ENDPOINT_NAME",
                        path.clone(),
                        format!(
                            "the request type `{}` must be named <Endpoint>Request",
                            e.input
                        ),
                    );
                    ""
                }
            };
            for role in &e.roles {
                if !(role.is_workspace() || declared_roles.contains(role.name())) {
                    err(
                        &mut errors,
                        "E_ENDPOINT_ROLE_UNKNOWN",
                        format!("{path}.roles"),
                        format!(
                            "role `{}` is neither declared by the plugin nor workspace.*",
                            role.name()
                        ),
                    );
                }
            }
            let scopes = e.scope_names();
            for scope in &scopes {
                if !known_scopes.contains(scope) {
                    err(
                        &mut errors,
                        "E_ENDPOINT_SCOPE_UNKNOWN",
                        format!("{path}.scopes"),
                        format!("scope `{scope}` is not declared"),
                    );
                }
            }
            if e.timeout < Duration::from_millis(1) || e.timeout > MAX_TIMEOUT {
                err(
                    &mut errors,
                    "E_ENDPOINT_TIMEOUT_RANGE",
                    format!("{path}.timeout"),
                    "the timeout is 1 ms to 600 s".into(),
                );
            }
            endpoints.push(EndpointDecl {
                name: name.to_string(),
                input: e.input.to_string(),
                output: e.output.to_string(),
                description: e.description.clone(),
                auth: e.auth.as_str().to_string(),
                roles: e.roles.iter().map(|r| r.name().to_string()).collect(),
                scopes: scopes.iter().map(|s| s.to_string()).collect(),
                idempotency: e.idempotency.as_str().to_string(),
                timeout_ms: e.timeout.as_millis() as u64,
                read_only: e.ui && e.read_only,
                ui: e.ui,
                invalidates: e.invalidates.iter().map(|i| i.json()).collect(),
            });
        }

        // the dependencies as declared, then those only read
        let mut dependencies: Vec<DependencyDecl> = self
            .dependencies
            .iter()
            .map(|(plugin, calls)| DependencyDecl {
                plugin: plugin.clone(),
                calls: calls.clone(),
                reads: vec![],
                lake: vec![],
            })
            .collect();
        for (plugin, reads) in &self.reads {
            let at = match dependencies.iter().position(|d| d.plugin == *plugin) {
                Some(i) => i,
                None => {
                    dependencies.push(DependencyDecl {
                        plugin: plugin.clone(),
                        calls: vec![],
                        reads: vec![],
                        lake: vec![],
                    });
                    dependencies.len() - 1
                }
            };
            dependencies[at].reads.extend(reads.iter().map(|r| {
                ReadDecl {
                    entity: r.entity.to_string(),
                    fields: r
                        .fields
                        .iter()
                        .map(|(name, ty)| ReadFieldDecl {
                            name: name.to_string(),
                            ty: ty.to_string(),
                        })
                        .collect(),
                }
            }));
        }

        if !self.notifications.is_empty()
            && !self
                .capabilities
                .iter()
                .any(|(c, _)| *c == Capability::PUSH_SEND)
        {
            err(
                &mut errors,
                "E_NOTIFY_CAPABILITY",
                "notifications".into(),
                "notification kinds need the capability push:send".into(),
            );
        }
        let mut seen = HashSet::new();
        for (i, k) in self.notifications.iter().enumerate() {
            if !(snake_ok(k.name()) && k.name().len() <= 32) {
                err(
                    &mut errors,
                    "E_NOTIFY_KIND_NAME",
                    format!("notifications[{i}]"),
                    format!("`{}` must match ^[a-z][a-z0-9_]{{0,31}}$", k.name()),
                );
            } else if !seen.insert(k.name()) {
                err(
                    &mut errors,
                    "E_NOTIFY_KIND_NAME",
                    format!("notifications[{i}]"),
                    format!("notification kind `{}` is declared twice", k.name()),
                );
            }
        }

        let mut views = vec![];
        for (i, (_, message, decl)) in self.views.iter().enumerate() {
            match decl {
                Ok(v) => {
                    let mut v = v.clone();
                    let row = reg.messages.iter().find(|m| m.name == *message);
                    v.description = row.map(|m| m.description.to_string()).unwrap_or_default();
                    // the columns in the order of the row's fields, whatever the order of the `col` calls (as the
                    // other SDKs write them)
                    if let Some(row) = row {
                        let at = |name: &str| {
                            row.fields
                                .iter()
                                .position(|f| f.name == name)
                                .unwrap_or(usize::MAX)
                        };
                        v.columns.sort_by_key(|c| at(&c.name));
                    }
                    views.push(v);
                }
                Err(e) => err(
                    &mut errors,
                    "E_VIEW_DEFINE",
                    format!("views[{i}]"),
                    e.clone(),
                ),
            }
        }

        // the lake: other plugins' tables read, the tables with their readers
        for (plugin, table) in &self.lake_deps {
            let at = match dependencies.iter().position(|d| d.plugin == *plugin) {
                Some(i) => i,
                None => {
                    dependencies.push(DependencyDecl {
                        plugin: plugin.to_string(),
                        calls: vec![],
                        reads: vec![],
                        lake: vec![],
                    });
                    dependencies.len() - 1
                }
            };
            if !dependencies[at].lake.iter().any(|t| t == table) {
                dependencies[at].lake.push(table.to_string());
            }
        }
        for (i, e) in self.lake_errors.iter().enumerate() {
            err(
                &mut errors,
                "E_LAKE_DEFINE",
                format!("lake_tables[{i}]"),
                e.clone(),
            );
        }
        let declared = |t: &str| self.lake_tables.iter().any(|(_, _, d)| d.name == t);
        for t in self
            .lake_reads
            .iter()
            .map(|(t, _)| *t)
            .chain(self.lake_system.iter().copied())
            .chain(self.lake_exports.iter().copied())
            .chain(self.lake_model_exports.iter().copied())
        {
            if !declared(t) {
                err(
                    &mut errors,
                    "E_LAKE_DEFINE",
                    format!("lake_tables[{t}]"),
                    format!(
                        "{t} is not a lake table of the plugin: declare it with lake_table first"
                    ),
                );
            }
        }
        let mut lake_models = vec![];
        for (i, m) in self.lake_models.iter().enumerate() {
            match m {
                Ok(m) => {
                    let mut m = m.clone();
                    // the columns in the order of the row's fields, as a view's
                    let message = self
                        .lake_tables
                        .iter()
                        .find(|(_, _, d)| d.name == m.table)
                        .map(|(_, message, _)| *message);
                    if let Some(row) =
                        message.and_then(|n| reg.messages.iter().find(|x| x.name == n))
                    {
                        let at = |name: &str| {
                            row.fields
                                .iter()
                                .position(|f| f.name == name)
                                .unwrap_or(usize::MAX)
                        };
                        m.columns.sort_by_key(|c| at(&c.name));
                    }
                    lake_models.push(m);
                }
                Err(e) => err(
                    &mut errors,
                    "E_LAKE_MODEL_DEFINE",
                    format!("lake_models[{i}]"),
                    e.clone(),
                ),
            }
        }
        let mut loads = vec![];
        for (i, l) in self.loads.iter().enumerate() {
            match l {
                Ok(l) => {
                    let mut l = l.clone();
                    // the columns in the order of the entity's fields, as a view's
                    if let Some(e) = reg.entities.iter().find(|e| e.name == l.entity) {
                        let at = |name: &str| {
                            e.fields
                                .iter()
                                .position(|f| f.name == name)
                                .unwrap_or(usize::MAX)
                        };
                        l.columns.sort_by_key(|c| at(&c.name));
                    }
                    loads.push(l);
                }
                Err(e) => err(
                    &mut errors,
                    "E_LOAD_DEFINE",
                    format!("loads[{i}]"),
                    e.clone(),
                ),
            }
        }
        let lake_tables: Vec<celobus_descriptor::lake::LakeTable> = self
            .lake_tables
            .iter()
            .map(|(_, message, d)| celobus_descriptor::lake::LakeTable {
                name: d.name.to_string(),
                layer: d.layer.to_string(),
                message: message.to_string(),
                description: reg
                    .messages
                    .iter()
                    .find(|m| m.name == *message)
                    .map(|m| m.description.to_string())
                    .unwrap_or_default(),
                partition: d.partition.iter().map(|c| c.to_string()).collect(),
                key: d.key.iter().map(|c| c.to_string()).collect(),
                read: roles_once(
                    self.lake_reads
                        .iter()
                        .filter(|(t, _)| *t == d.name)
                        .flat_map(|(_, r)| r.iter().cloned()),
                ),
                system_read: self.lake_system.contains(&d.name),
                export: self.lake_exports.contains(&d.name),
                export_models: self.lake_model_exports.contains(&d.name),
            })
            .collect();

        if self.job_slots == 0 || self.job_slots > crate::jobs::MAX_JOB_SLOTS {
            err(
                &mut errors,
                "E_JOB_SLOTS",
                "job_slots".into(),
                format!(
                    "{} job slots; a process runs 1 to {} runs at once",
                    self.job_slots,
                    crate::jobs::MAX_JOB_SLOTS
                ),
            );
        }

        if !errors.is_empty() {
            return Err(DescribeErrors(errors));
        }
        let mut d = Descriptor {
            schema: 1,
            id: self.id.clone(),
            version: self.version.clone(),
            title: self.title.clone().unwrap_or_else(|| humanize(&self.id)),
            description: self.description.clone(),
            roles: self
                .roles
                .iter()
                .map(|r| RoleDecl {
                    name: r.name().into(),
                    title: r.get_title().into(),
                    description: r.description().into(),
                    strict: r.is_strict(),
                    default_for: r.get_default_for().as_str().into(),
                })
                .collect(),
            scopes: self
                .scopes
                .iter()
                .map(|s| ScopeDecl {
                    name: s.name().into(),
                    title: s.get_title().into(),
                    description: s.description().into(),
                    sensitive: s.is_sensitive(),
                })
                .collect(),
            capabilities: self
                .capabilities
                .iter()
                .map(|(c, reason)| CapabilityDecl {
                    name: c.name().into(),
                    reason: reason.clone(),
                    on: if *c == Capability::AUTHZ_BYPASS {
                        self.bypass.clone()
                    } else {
                        vec![]
                    },
                })
                .collect(),
            permissions: self.permissions.clone(),
            agents: self
                .agents
                .iter()
                .map(|a| AgentDecl {
                    name: a.name.clone(),
                    roles: a.roles.clone(),
                    workspace: a.workspace.clone(),
                    delegates: a.delegates.clone(),
                    connection: a.connection.clone(),
                    ssh_key: a.ssh_key,
                    admin: a.admin,
                    title: a.title.clone(),
                    description: a.description.clone(),
                    icon: a.icon.clone(),
                    engine: a.engine.map(|e| e.name().to_string()).unwrap_or_default(),
                    model: a.model.clone(),
                    session: a.session,
                    timeout_s: a.timeout.map(|t| t.as_secs()),
                    skills: a.skills.clone(),
                    topic: a.topic.clone(),
                })
                .collect(),
            connections: self
                .connections
                .iter()
                .map(|c| ConnectionDecl {
                    name: c.name.clone(),
                    kind: c.kind.name().into(),
                    title: c.title.clone(),
                    description: c.description.clone(),
                })
                .collect(),
            dependencies,
            enums,
            messages,
            entities,
            endpoints,
            topics: self.topics.iter().map(|t| t.json()).collect(),
            ui: self
                .ui
                .as_ref()
                .map(|u| serde_json::to_value(u).expect("the UI serializes")),
            notifications: self
                .notifications
                .iter()
                .map(|k| NotificationDecl {
                    name: k.name().into(),
                    title: k.get_title().into(),
                    description: k.description().into(),
                    tab: k.tab().as_str().into(),
                    digest: k.in_digest(),
                })
                .collect(),
            runs: self
                .on_run_change
                .as_ref()
                .map(|_| RunsDecl { on_change: true }),
            jobs: self.jobs.iter().map(|j| j.descriptor()).collect(),
            views,
            layers: self
                .layers
                .iter()
                .map(|l| celobus_descriptor::lake::Layer {
                    name: l.name().to_string(),
                    title: l.get_title().to_string(),
                })
                .collect(),
            lake_tables,
            lake_models,
            loads,
            settings: None,
        };
        d.settings = self.settings.as_ref().map(|s| settings_section(s, &d));
        // upload inputs carry their file field's options, so a screen checks a file before uploading it
        if let Some(ui) = d.ui.as_mut() {
            fill_uploads(ui, &d.endpoints, &d.messages);
        }
        // `history(.., all)`: the flag on every column it covers (the descriptor carries one per column), worked out
        // by the descriptor's own rule (the encrypted columns left out, and the masked ones without `unmasked`)
        if d.entities
            .iter()
            .any(|e| e.history.as_ref().is_some_and(|h| h.all))
            && let Ok(parsed) = celobus_descriptor::parse(d.to_canonical_json().as_bytes())
        {
            for e in d.entities.iter_mut() {
                let Some(pe) = parsed.entities.iter().find(|x| x.name == e.name) else {
                    continue;
                };
                let covered = celobus_descriptor::history::covered(&parsed, pe);
                for f in e.fields.iter_mut() {
                    if covered.contains(&f.name) {
                        f.history = true;
                    }
                }
            }
        }
        // the jobs', the views' and the lake's rules are the package's own (E_JOB_*, E_VIEW_*, E_LAKE_*, E_HISTORY_*):
        // the same check runs at build and at install
        if (!d.jobs.is_empty()
            || !d.views.is_empty()
            || !d.lake_tables.is_empty()
            || !d.lake_models.is_empty()
            || !d.layers.is_empty()
            || !d.loads.is_empty()
            || d.settings.is_some()
            || d.entities
                .iter()
                .any(|e| e.history.is_some() || !e.storage.is_empty()))
            && let Err(errors) = celobus_descriptor::check(d.to_canonical_json().as_bytes())
        {
            let jobs: Vec<DescribeError> = errors
                .into_iter()
                .filter(|e| {
                    e.code.starts_with("E_JOB_")
                        || e.code.starts_with("E_VIEW_")
                        || e.code.starts_with("E_LAKE_")
                        || e.code.starts_with("E_HISTORY_")
                        || e.code.starts_with("E_OLAP_")
                        || e.code.starts_with("E_LOAD_")
                        || e.code.starts_with("E_SETTINGS_")
                        || e.code == "E_FIELD_VALIDATOR_SCOPE"
                })
                .map(|e| {
                    let message = match e.help {
                        Some(help) => format!("{} ({help})", e.message),
                        None => e.message,
                    };
                    DescribeError::new(e.code, e.path, message)
                })
                .collect();
            if !jobs.is_empty() {
                return Err(DescribeErrors(jobs));
            }
        }
        Ok(d)
    }

    /// Runs the plugin and blocks: with `--describe` among the arguments it prints the descriptor and
    /// exits 0; on definition errors it prints them (`error[E_CODE] at <path>: <message>`) and exits 2;
    /// otherwise it connects to the platform at `$CELOBUS_ADDR` on its own tokio runtime,
    /// serves until SIGTERM (or Ctrl-C), and returns.
    pub fn serve(self) {
        let describe = std::env::args().skip(1).any(|a| a == "--describe");
        match self.descriptor() {
            Err(errors) => {
                eprintln!("{errors}");
                std::process::exit(2);
            }
            Ok(d) if describe => {
                let mut out = std::io::stdout().lock();
                let _ = out.write_all(d.to_canonical_json().as_bytes());
                let _ = out.flush();
                std::process::exit(0);
            }
            Ok(_) => {}
        }
        crate::session::init_logging();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("start the tokio runtime");
        if let Err(e) = runtime.block_on(self.serve_async()) {
            tracing::error!(error = %format!("{e:#}"), "the plugin stopped");
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    }

    /// `serve()` for a program that has its own tokio runtime (no `--describe`, no logging setup).
    pub async fn serve_async(self) -> anyhow::Result<()> {
        let addr = std::env::var("CELOBUS_ADDR")
            .map_err(|_| anyhow::anyhow!("CELOBUS_ADDR is not set (the platform sets it)"))?;
        let service = crate::session::Service::new(self).map_err(|e| anyhow::anyhow!("{e}"))?;
        crate::session::serve_until(
            std::sync::Arc::new(service),
            &addr,
            crate::session::shutdown_signal(),
        )
        .await;
        Ok(())
    }
}

/// The descriptor's `settings`: the message, whether the plugin checks a change, and the form — laid out by the plugin
/// and completed from the fields, or derived from them (`celobus_descriptor::settings`, the rule every SDK follows).
fn settings_section(
    s: &crate::settings::SettingsDecl,
    d: &Descriptor,
) -> celobus_descriptor::settings::Settings {
    use celobus_descriptor::settings::{SettingsForm, complete_form, derive_form};
    let fields: Vec<celobus_descriptor::Field> = d
        .messages
        .iter()
        .find(|m| m.name == s.message)
        .map(|m| {
            m.fields
                .iter()
                .filter_map(|f| serde_json::to_value(f).ok())
                .filter_map(|v| serde_json::from_value(v).ok())
                .collect()
        })
        .unwrap_or_default();
    let enums: Vec<celobus_descriptor::Enum> = d
        .enums
        .iter()
        .map(|e| celobus_descriptor::Enum {
            name: e.name.clone(),
            values: e.values.clone(),
        })
        .collect();
    let form = match &s.form {
        Some(sections) => complete_form(
            SettingsForm {
                sections: sections.clone(),
            },
            &fields,
            &enums,
        ),
        None => derive_form(&fields, &enums),
    };
    celobus_descriptor::settings::Settings {
        message: s.message.to_string(),
        on_change: s.on_change.is_some(),
        form,
    }
}

fn plugin_id_ok(id: &str) -> bool {
    let b = id.as_bytes();
    (2..=16).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// Role and scope keys: `^[a-z][a-z0-9_]{1,31}$`.
/// snake_case: `^[a-z][a-z0-9_]{0,62}$`.
fn snake_ok(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=63).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

fn key_ok(key: &str) -> bool {
    let b = key.as_bytes();
    (2..=32).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

fn semver_ok(v: &str) -> bool {
    let (core, rest) = match v.find(['-', '+']) {
        Some(i) => (&v[..i], &v[i..]),
        None => (v, ""),
    };
    let parts: Vec<&str> = core.split('.').collect();
    let numeric = |p: &&str| {
        !p.is_empty()
            && p.bytes().all(|c| c.is_ascii_digit())
            && (p.len() == 1 || !p.starts_with('0'))
    };
    let ident_ok = |s: &str| {
        s.split('.')
            .all(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'))
    };
    let rest_ok = if rest.is_empty() {
        true
    } else {
        let (pre, build) = match rest.find('+') {
            Some(i) => (&rest[..i], Some(&rest[i + 1..])),
            None => (rest, None),
        };
        let pre_ok = pre.is_empty() || pre.strip_prefix('-').is_some_and(ident_ok);
        pre_ok && build.is_none_or(ident_ok)
    };
    parts.len() == 3 && parts.iter().all(numeric) && rest_ok
}

/// Copies each `upload` input's file field options into it (`multiple` for a list of files, `max_size`, `accept`),
/// from the request of its form's endpoint, inputs of a `repeat` included.
fn fill_uploads(ui: &mut serde_json::Value, endpoints: &[EndpointDecl], messages: &[MessageDecl]) {
    use serde_json::Value;
    match ui {
        Value::Object(m) => {
            if let Some(Value::Object(form)) = m.get_mut("form")
                && let Some(input) = form
                    .get("endpoint")
                    .and_then(Value::as_str)
                    .and_then(|e| endpoints.iter().find(|x| x.name == e))
                    .map(|e| e.input.clone())
                && let Some(Value::Array(sections)) = form.get_mut("sections")
            {
                for s in sections {
                    if let Some(Value::Array(inputs)) = s.get_mut("inputs") {
                        fill_upload_inputs(inputs, &input, messages);
                    }
                }
            }
            for v in m.values_mut() {
                fill_uploads(v, endpoints, messages);
            }
        }
        Value::Array(a) => {
            for v in a {
                fill_uploads(v, endpoints, messages);
            }
        }
        _ => {}
    }
}

fn fill_upload_inputs(inputs: &mut [serde_json::Value], msg: &str, messages: &[MessageDecl]) {
    use serde_json::Value;
    for i in inputs {
        let Some(f) = i
            .get("name")
            .and_then(Value::as_str)
            .and_then(|name| field_at(messages, msg, name))
        else {
            continue;
        };
        if i.get("control").and_then(Value::as_str) == Some("upload") && f.ty == "file" {
            if f.repeated {
                i["multiple"] = Value::Bool(true);
            }
            if let Some(n) = f.max_size {
                i["max_size"] = Value::from(n);
            }
            if !f.accept.is_empty() {
                i["accept"] = Value::from(f.accept.clone());
            }
        }
        if let Some(m) = f.ty.strip_prefix("message:")
            && let Some(Value::Array(nested)) = i.get_mut("fields")
        {
            fill_upload_inputs(nested, m, messages);
        }
    }
}

/// A field of a message by its path (`customer.name`), through nested messages.
fn field_at<'a>(messages: &'a [MessageDecl], msg: &str, path: &str) -> Option<&'a FieldDecl> {
    let (head, rest) = match path.split_once('.') {
        Some((h, r)) => (h, Some(r)),
        None => (path, None),
    };
    let f = messages
        .iter()
        .find(|m| m.name == msg)?
        .fields
        .iter()
        .find(|f| f.name == head)?;
    match rest {
        None => Some(f),
        Some(r) => field_at(messages, f.ty.strip_prefix("message:")?, r),
    }
}

fn humanize(id: &str) -> String {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Read roles as the descriptor lists them, a lake table's and a history's alike: each once, where it was first named.
fn roles_once(roles: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for r in roles {
        if !out.contains(&r) {
            out.push(r);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        for ok in ["0.1.0", "1.2.3-dev.1", "1.0.0+build.5", "10.20.30-rc-1+x"] {
            assert!(semver_ok(ok), "{ok}");
        }
        for bad in ["1.2", "01.2.3", "1.2.3-", "a.b.c", ""] {
            assert!(!semver_ok(bad), "{bad}");
        }
    }

    #[test]
    fn ids() {
        assert!(plugin_id_ok("hello"));
        assert!(!plugin_id_ok("h"));
        assert!(!plugin_id_ok("Hello"));
        assert!(!plugin_id_ok("hello_world"));
        assert!(key_ok("clerk"));
        assert!(!key_ok("workspace.admin"));
    }
}
