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
    pub(crate) endpoints: Vec<Endpoint>,
    topics: Vec<crate::ui::TopicDecl>,
    ui: Option<crate::ui::doc::Ui>,
    /// the run-change hook (`on_run_change`)
    pub(crate) on_run_change: Option<crate::endpoint::Handler>,
    pub(crate) jobs: Vec<crate::jobs::JobDecl>,
    /// how many runs the process starts at once
    pub(crate) job_slots: u32,
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
            endpoints: Vec::new(),
            topics: Vec::new(),
            ui: None,
            on_run_change: None,
            jobs: Vec::new(),
            job_slots: crate::jobs::DEFAULT_JOB_SLOTS,
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
            });
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
        let d = Descriptor {
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
        };
        // the jobs' rules are the package's own (E_JOB_*): the same check runs at build and at install
        if !d.jobs.is_empty()
            && let Err(errors) = celobus_descriptor::check(d.to_canonical_json().as_bytes())
        {
            let jobs: Vec<DescribeError> = errors
                .into_iter()
                .filter(|e| e.code.starts_with("E_JOB_"))
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

fn humanize(id: &str) -> String {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
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
