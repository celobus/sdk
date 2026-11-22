//! `Ctx`: what a handler gets with its request. The caller, the request, the plugin, calls to
//! dependencies, the plugin's entities and declared transactions (`db`, `transaction`: `crate::db`), reads of a
//! dependency's entities and declared SQL (`query`, `sql`: `crate::sql`), the system context (`as_system`), the
//! workspace directory, the runs of the plugin's agents, notifications (`notifications`: `crate::notify`), background
//! jobs (`jobs`, and `job` in a job's handler: `crate::jobs`); `events` answer `unavailable`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use celobus_proto as pb;
use pb::{plugin_message, syscall, syscall_result};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::error::{Error, Result};
use crate::identity::{Role, Scope};
use crate::message::{Call, Message};
use crate::platform::{Group, User};
use crate::types::Id;

const EVENTS_UNAVAILABLE: &str = "events are not available";

/// The admission refusals the platform answers a syscall with (`<code>: <message>`), as 403 with that code: an
/// invoke of a dependency endpoint the plugin did not declare is `call_undeclared`, one the caller may not use
/// `missing_role` or `missing_scope`.
pub(crate) const DENIALS: &[&str] = &[
    "call_undeclared",
    "missing_role",
    "missing_scope",
    "reach",
    "not_a_member",
];

/// What the platform refuses an `agent_*` syscall with, and the status each code answers: an agent, a run or a group
/// the plugin does not have is 404, a run that ended the platform's `run_ended` conflict, and so are a group that
/// has a live run of the agent already (`agent_in_group`) or its eight live runs (`group_full`).
pub(crate) const AGENT_REFUSALS: &[(&str, u16)] = &[
    ("unknown_agent", 404),
    ("not_found", 404),
    ("unknown_group", 404),
    ("run_ended", 409),
    ("agent_in_group", 409),
    ("group_full", 409),
    ("bad_request", 400),
    ("forbidden", 403),
];

/// What the platform refuses a `Db*` or `Tx*` syscall with, and the status each code answers.
pub(crate) const DB_REFUSALS: &[(&str, u16)] = &[
    ("not_found", 404),
    ("version_conflict", 409),
    ("unique_conflict", 409),
    ("transaction_conflict", 409),
    ("row_locked", 409),
    ("append_only", 400),
    ("invalid", 400),
    ("query_rows_limit", 422),
    ("bulk_limit", 422),
    ("transaction_timeout", 504),
    ("permission_denied", 403),
    ("guard_denied", 403),
    ("capability_missing", 403),
    ("query_invalid", 500),
    ("query_size_limit", 422),
    ("query_join_limit", 422),
    ("query_response_limit", 422),
];

/// A db refusal as the typed error: `version_conflict` carries `details.current`, `unique_conflict`
/// `details.column`.
fn db_refusal(error: &str, details: Option<&pb::DbErrorDetails>) -> Option<Error> {
    let (code, message) = error.split_once(": ")?;
    let (code, status) = DB_REFUSALS.iter().find(|(c, _)| *c == code)?;
    let mut e = Error::new(*status, *code, message);
    if let Some(d) = details {
        if *code == "version_conflict" {
            e = e.detail("current", d.current_version);
        }
        if *code == "unique_conflict" {
            e = e.detail("column", d.column.clone());
        }
    }
    Some(e)
}

/// What the platform refuses a `notify_*` syscall with besides the db codes (`invalid`, `capability_missing`).
pub(crate) const NOTIFY_REFUSALS: &[(&str, u16)] = &[("follow_limit", 409), ("rate_limited", 429)];

/// What the platform refuses a job call with besides the db codes (`invalid`, `not_found`).
pub(crate) const JOB_REFUSALS: &[(&str, u16)] = &[("queue_full", 429), ("payload_too_large", 413)];

fn denied(error: &str) -> Option<Error> {
    let (code, message) = error.split_once(": ")?;
    if let Some(code) = DENIALS.iter().find(|c| **c == code) {
        return Some(Error::new(403, *code, message));
    }
    let (code, status) = AGENT_REFUSALS
        .iter()
        .chain(NOTIFY_REFUSALS)
        .chain(JOB_REFUSALS)
        .find(|(c, _)| *c == code)?;
    Some(Error::new(*status, *code, message))
}

/// The live session with the platform, shared by every `Ctx` of it: syscalls go out on `tx` and their
/// results come back through `pending`.
pub(crate) struct Link {
    tx: mpsc::Sender<pb::PluginMessage>,
    pending: Mutex<HashMap<u64, oneshot::Sender<pb::SyscallResult>>>,
    next: AtomicU64,
}

impl Link {
    pub(crate) fn new(tx: mpsc::Sender<pb::PluginMessage>) -> Self {
        Self {
            tx,
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
        }
    }

    /// A link to no session: every syscall answers `unavailable` (unit tests of handlers).
    pub(crate) fn detached() -> Self {
        let (tx, _) = mpsc::channel(1);
        Self::new(tx)
    }

    /// Delivers a syscall result to the call waiting for it.
    pub(crate) fn complete(&self, result: pb::SyscallResult) {
        let waiter = self
            .pending
            .lock()
            .expect("pending syscalls")
            .remove(&result.id);
        if let Some(waiter) = waiter {
            let _ = waiter.send(result);
        }
    }

    pub(crate) async fn syscall(
        &self,
        call: syscall::Call,
    ) -> Result<Option<syscall_result::Result>> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending syscalls")
            .insert(id, tx);
        let msg = pb::PluginMessage {
            kind: Some(plugin_message::Kind::Syscall(pb::Syscall {
                id,
                call: Some(call),
            })),
        };
        if self.tx.send(msg).await.is_err() {
            self.pending.lock().expect("pending syscalls").remove(&id);
            return Err(Error::unavailable(
                "the session with the platform is closed",
            ));
        }
        let res = rx.await.map_err(|_| {
            Error::unavailable("the session with the platform closed during the call")
        })?;
        if !res.error.is_empty() {
            return Err(match res.error.as_str() {
                "not found" => Error::not_found(res.error),
                _ => db_refusal(&res.error, res.error_details.as_ref())
                    .or_else(|| denied(&res.error))
                    .unwrap_or_else(|| Error::unavailable(res.error)),
            });
        }
        Ok(res.result)
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        // waiters see their channel closed: `unavailable`
        if let Ok(mut pending) = self.pending.lock() {
            pending.clear();
        }
    }
}

/// What a handler gets: cheap to clone (an `Arc`).
#[derive(Clone)]
pub struct Ctx {
    inner: Arc<CtxInner>,
    /// the declared transaction its repository calls are part of (0: none)
    tx: u64,
    /// the reason of a system context (`as_system`); none: the caller's
    system: Option<Arc<str>>,
}

struct CtxInner {
    link: Arc<Link>,
    plugin: Arc<PluginInfo>,
    request: RequestInfo,
    caller: Caller,
    /// the run, in a job's handler
    job: Option<crate::jobs::JobInfo>,
}

/// The plugin serving the request.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    id: String,
    version: String,
    /// its jobs' keys by payload message; `None` for a test context, which takes the payload's name in snake_case
    jobs: Option<HashMap<&'static str, String>>,
}

impl PluginInfo {
    pub(crate) fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            jobs: None,
        }
    }

    /// With the plugin's declared jobs: (payload message, job key).
    pub(crate) fn with_jobs(
        mut self,
        jobs: impl IntoIterator<Item = (&'static str, String)>,
    ) -> Self {
        self.jobs = Some(jobs.into_iter().collect());
        self
    }

    /// The key of the job whose payload is `message`.
    pub(crate) fn job_for(&self, message: &'static str) -> Result<String> {
        match &self.jobs {
            None => Ok(crate::jobs::job_key(message)),
            Some(jobs) => jobs.get(message).cloned().ok_or_else(|| {
                Error::invalid(format!(
                    "no job of the plugin takes {message}: declare one with Plugin::job(Job::new(..))"
                ))
            }),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

/// The request being handled.
#[derive(Debug, Clone)]
pub struct RequestInfo {
    id: u64,
    deadline: Instant,
    idempotency_key: Option<String>,
}

impl RequestInfo {
    pub(crate) fn new(id: u64, timeout: Duration, idempotency_key: Option<String>) -> Self {
        Self {
            id,
            deadline: Instant::now() + timeout,
            idempotency_key,
        }
    }

    /// The platform's id of the request (dependency calls carry it).
    pub fn id(&self) -> u64 {
        self.id
    }

    /// When the endpoint's timeout passes; pass it to the plugin's own I/O.
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// The `Idempotency-Key` header, when the caller sent one.
    pub fn idempotency_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }
}

/// What kind of principal calls: a person, a service account, or the system (no member).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CallerKind {
    User,
    Service,
    #[default]
    System,
}

/// Who calls, resolved by the platform from the workspace's members (never by the plugin).
#[derive(Debug, Clone, Default)]
pub struct Caller {
    user_id: Option<Id<User>>,
    kind: CallerKind,
    display_name: String,
    roles: Vec<String>,
    groups: Vec<Id<Group>>,
    scopes: Vec<String>,
    plugin: String,
}

impl Caller {
    pub(crate) fn from_principal(p: Option<pb::Principal>, plugin: &str) -> Self {
        let Some(p) = p else {
            return Self {
                plugin: plugin.into(),
                ..Default::default()
            };
        };
        let kind = match pb::principal::Kind::try_from(p.kind) {
            Ok(pb::principal::Kind::Person) => CallerKind::User,
            Ok(pb::principal::Kind::Service) => CallerKind::Service,
            _ if p.user_id.is_empty() => CallerKind::System,
            _ => CallerKind::User,
        };
        Self {
            user_id: (!p.user_id.is_empty()).then(|| Id::new(p.user_id)),
            kind,
            display_name: p.display_name,
            roles: p.roles,
            groups: p.groups.into_iter().map(Id::new).collect(),
            scopes: p.scopes,
            plugin: plugin.into(),
        }
    }

    /// A caller for unit tests of handlers: a person with this id and no roles.
    pub fn test_user(user_id: impl Into<String>) -> Self {
        Self {
            user_id: Some(Id::new(user_id.into())),
            kind: CallerKind::User,
            display_name: "Test User".into(),
            ..Default::default()
        }
    }

    /// Adds full role names (`workspace.admin`, `<plugin>.<role>`); for tests.
    pub fn with_roles<S: Into<String>>(mut self, roles: impl IntoIterator<Item = S>) -> Self {
        self.roles.extend(roles.into_iter().map(Into::into));
        self
    }

    /// Adds full scope names (`<plugin>:<scope>`); for tests.
    pub fn with_scopes<S: Into<String>>(mut self, scopes: impl IntoIterator<Item = S>) -> Self {
        self.scopes.extend(scopes.into_iter().map(Into::into));
        self
    }

    /// The calling user, or 401 `unauthenticated` when nobody is signed in.
    pub fn user(&self) -> Result<Id<User>> {
        self.user_id
            .clone()
            .ok_or_else(|| Error::unauthenticated("this request has no signed-in caller"))
    }

    pub fn user_id(&self) -> Option<&Id<User>> {
        self.user_id.as_ref()
    }

    pub fn kind(&self) -> CallerKind {
        self.kind
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Effective roles: `workspace.*` (expanded) and plugin roles `<plugin>.<role>`.
    pub fn roles(&self) -> &[String] {
        &self.roles
    }

    /// Holds the role: one of this plugin's (`MANAGER` → `<plugin>.manager`) or `Role::WORKSPACE_*`.
    pub fn has(&self, role: &Role) -> bool {
        let full = role.full_name(&self.plugin);
        self.roles.contains(&full)
    }

    pub fn groups(&self) -> &[Id<Group>] {
        &self.groups
    }

    /// The token's scopes for this plugin, `<plugin>:<scope>`.
    pub fn scopes(&self) -> &[String] {
        &self.scopes
    }

    pub fn has_scope(&self, scope: &Scope) -> bool {
        let full = scope.full_name(&self.plugin);
        self.scopes.contains(&full)
    }
}

/// `/<plugin>.v1.<Service>/<Name>`; the service is the id in PascalCase.
pub(crate) fn endpoint_path(plugin: &str, name: &str) -> String {
    format!("/{plugin}.v1.{}/{name}", service_name(plugin))
}

pub(crate) fn service_name(plugin: &str) -> String {
    plugin
        .split(['_', '-'])
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

impl Ctx {
    pub(crate) fn new(
        link: Arc<Link>,
        plugin: Arc<PluginInfo>,
        request: RequestInfo,
        caller: Caller,
    ) -> Self {
        Self {
            inner: Arc::new(CtxInner {
                link,
                plugin,
                request,
                caller,
                job: None,
            }),
            tx: 0,
            system: None,
        }
    }

    /// The context of a job's run.
    pub(crate) fn for_run(
        link: Arc<Link>,
        plugin: Arc<PluginInfo>,
        request: RequestInfo,
        caller: Caller,
        job: crate::jobs::JobInfo,
    ) -> Self {
        Self {
            inner: Arc::new(CtxInner {
                link,
                plugin,
                request,
                caller,
                job: Some(job),
            }),
            tx: 0,
            system: None,
        }
    }

    /// The run, in a job's handler: its id, attempt, attempts allowed and scheduled time. Outside a job's run: an
    /// empty run id and attempt 0 ([`crate::jobs::JobInfo::is_run`]).
    pub fn job(&self) -> &crate::jobs::JobInfo {
        self.inner
            .job
            .as_ref()
            .unwrap_or_else(|| crate::jobs::JobInfo::none())
    }

    /// The transaction this context's repository calls are part of (0: none).
    pub(crate) fn tx(&self) -> u64 {
        self.tx
    }

    /// The same context inside a declared transaction.
    pub(crate) fn in_transaction(&self, tx: u64) -> Self {
        Self {
            inner: self.inner.clone(),
            tx,
            system: self.system.clone(),
        }
    }

    /// The same context with the system's rights on the plugin's own entities: its repository calls (and the
    /// transactions it begins) bypass the access rules within the scope the plugin declares for `authz:bypass`
    /// (`plugin.bypass::<E>([..])`; outside it, 403 `capability_missing`), read masked columns, and are recorded in
    /// the workspace's audit with the caller, the request and `reason`.
    pub fn as_system(&self, reason: &str) -> Self {
        Self {
            inner: self.inner.clone(),
            tx: self.tx,
            system: Some(reason.into()),
        }
    }

    /// The reason of the system context, empty for the caller's own.
    pub(crate) fn system(&self) -> String {
        self.system.as_deref().unwrap_or_default().to_string()
    }

    /// A context without a session, for unit tests of handlers: the caller is the system (see
    /// [`Ctx::with_caller`]) and every call to the platform answers `unavailable`.
    pub fn test(plugin: &str, version: &str) -> Self {
        Self::new(
            Arc::new(Link::detached()),
            Arc::new(PluginInfo::new(plugin, version)),
            RequestInfo::new(0, Duration::from_secs(30), None),
            Caller::from_principal(None, plugin),
        )
    }

    /// The same context with another caller (unit tests).
    pub fn with_caller(&self, mut caller: Caller) -> Self {
        caller.plugin = self.inner.plugin.id.clone();
        Self {
            inner: Arc::new(CtxInner {
                link: self.inner.link.clone(),
                plugin: self.inner.plugin.clone(),
                request: self.inner.request.clone(),
                caller,
                job: self.inner.job.clone(),
            }),
            tx: self.tx,
            system: self.system.clone(),
        }
    }

    /// The same context as a job's run (unit tests of job handlers): `ctx.job()` answers this run id and attempt,
    /// scheduled now.
    pub fn with_run(&self, run_id: impl Into<String>, attempt: u32, max_attempts: u32) -> Self {
        Self {
            inner: Arc::new(CtxInner {
                link: self.inner.link.clone(),
                plugin: self.inner.plugin.clone(),
                request: self.inner.request.clone(),
                caller: self.inner.caller.clone(),
                job: Some(crate::jobs::JobInfo {
                    job: String::new(),
                    run_id: run_id.into(),
                    attempt,
                    max_attempts,
                    scheduled_at: chrono::Utc::now(),
                }),
            }),
            tx: self.tx,
            system: self.system.clone(),
        }
    }

    pub fn caller(&self) -> &Caller {
        &self.inner.caller
    }

    pub fn request(&self) -> &RequestInfo {
        &self.inner.request
    }

    pub fn plugin(&self) -> &PluginInfo {
        &self.inner.plugin
    }

    /// When the endpoint's timeout passes (`ctx.request().deadline()`).
    pub fn deadline(&self) -> Instant {
        self.inner.request.deadline
    }

    /// Calls an endpoint of a dependency, as this request's caller: the request type names the
    /// endpoint (`GetCustomerRequest` → `GetCustomer`) and its `#[message(response = ..)]` the answer.
    /// The dependency's error comes back as it answered (status, code, details).
    pub async fn call<R: Call>(&self, plugin: &str, req: &R) -> Result<R::Response> {
        let name = R::NAME
            .strip_suffix("Request")
            .filter(|n| !n.is_empty())
            .ok_or_else(|| {
                Error::internal(format!(
                    "{} does not name an endpoint (no `Request` suffix)",
                    R::NAME
                ))
            })?;
        let body = self.call_raw(plugin, name, req.to_json()).await?;
        R::Response::from_json(&body, "").map_err(|e| {
            Error::internal(format!(
                "{plugin}.{name} answered a body this plugin cannot read: {e}"
            ))
        })
    }

    /// Calls a dependency's endpoint by name with typed messages: what the clients `celobus plugin add`
    /// generates (`src/deps/`) call, `deps::catalog::get_product(&ctx, &req)`.
    pub async fn invoke<Req: Message, Resp: Message>(
        &self,
        plugin: &str,
        endpoint: &str,
        req: &Req,
    ) -> Result<Resp> {
        let body = self.call_raw(plugin, endpoint, req.to_json()).await?;
        Resp::from_json(&body, "").map_err(|e| {
            Error::internal(format!(
                "{plugin}.{endpoint} answered a body this plugin cannot read: {e}"
            ))
        })
    }

    /// Calls an endpoint of a dependency by name with a JSON body (protobuf JSON mapping).
    pub async fn call_raw(&self, plugin: &str, endpoint: &str, body: Value) -> Result<Value> {
        let request = pb::Request {
            method: "POST".into(),
            path: endpoint_path(plugin, endpoint),
            headers: vec![pb::Header {
                name: "content-type".into(),
                value: "application/json".into(),
            }],
            body: Bytes::from(serde_json::to_vec(&body)?),
            ..Default::default()
        };
        let invoke = pb::Invoke {
            plugin: plugin.into(),
            request: Some(request),
            request_id: self.inner.request.id,
        };
        let resp = match self
            .inner
            .link
            .syscall(syscall::Call::Invoke(invoke))
            .await?
        {
            Some(syscall_result::Result::InvokeResponse(r)) => r,
            _ => {
                return Err(Error::internal(
                    "the platform answered an invoke with something else",
                ));
            }
        };
        if !(200..300).contains(&resp.status) {
            return Err(Error::from_answer(resp.status as u16, &resp.body));
        }
        if resp.body.is_empty() {
            return Ok(Value::Object(Default::default()));
        }
        serde_json::from_slice(&resp.body)
            .map_err(|e| Error::internal(format!("{plugin}.{endpoint} answered invalid JSON: {e}")))
    }

    /// The workspace's members.
    pub fn members(&self) -> Members<'_> {
        Members { ctx: self }
    }

    /// The workspace's groups.
    pub fn groups(&self) -> Groups<'_> {
        Groups { ctx: self }
    }

    /// The runs of this plugin's agents: start one, read it and its events, write to it, stop it.
    pub fn agents(&self) -> crate::agents::Agents<'_> {
        crate::agents::Agents::new(self)
    }

    /// A syscall of this request's session.
    pub(crate) async fn syscall(
        &self,
        call: syscall::Call,
    ) -> Result<Option<syscall_result::Result>> {
        self.inner.link.syscall(call).await
    }

    /// Events: not available, `emit` answers `unavailable`.
    pub fn events(&self) -> Events {
        Events { _private: () }
    }

    /// The plugin's background jobs: `enqueue` a run, `cancel` one. On a declared transaction's context
    /// (`tx.jobs()`) an enqueue is part of the transaction.
    pub fn jobs(&self) -> crate::jobs::Jobs {
        crate::jobs::Jobs::new(self.clone())
    }

    /// Tells the screens following `topic` for `key` to re-read (plugin UI). An endpoint that declares
    /// `.invalidates(..)` needs no call: the platform emits those after its 2xx answer. This is for the rest: a change
    /// made by a job, or a key the endpoint's response does not carry. Sent once the session has room; nothing is
    /// awaited, and a screen that misses it re-reads within its maximum data age.
    pub fn invalidate<K: Message>(&self, topic: crate::ui::Topic<K>, key: &K) {
        let msg = pb::PluginMessage {
            kind: Some(plugin_message::Kind::Invalidate(pb::Invalidate {
                topic: topic.name().into(),
                key: canonical_key(&key.to_json()),
            })),
        };
        let _ = self.inner.link.tx.try_send(msg);
    }

    /// Every key of `topic` re-reads.
    pub fn invalidate_all<K: Message>(&self, topic: crate::ui::Topic<K>) {
        let msg = pb::PluginMessage {
            kind: Some(plugin_message::Kind::Invalidate(pb::Invalidate {
                topic: topic.name().into(),
                key: String::new(),
            })),
        };
        let _ = self.inner.link.tx.try_send(msg);
    }
}

/// A topic key as the platform and the clients match it: the key message's fields by their snake_case names, sorted,
/// values as strings (`{"id":"1048"}`); an empty key message is "" (every key).
pub(crate) fn canonical_key(json: &Value) -> String {
    let Some(o) = json.as_object() else {
        return String::new();
    };
    let mut fields: Vec<(String, String)> = o
        .iter()
        .filter_map(|(k, v)| {
            let v = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => return None,
            };
            Some((snake(k), v))
        })
        .collect();
    if fields.is_empty() {
        return String::new();
    }
    fields.sort();
    let m: serde_json::Map<String, Value> = fields
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    Value::Object(m).to_string()
}

fn snake(camel: &str) -> String {
    let mut out = String::with_capacity(camel.len() + 4);
    for c in camel.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn user_of(m: pb::Member) -> User {
    User {
        id: Id::new(m.user_id),
        kind: match pb::principal::Kind::try_from(m.kind) {
            Ok(pb::principal::Kind::Service) => CallerKind::Service,
            _ => CallerKind::User,
        },
        display_name: m.display_name,
        roles: m.roles,
        groups: m.groups.into_iter().map(Id::new).collect(),
        email: m.email,
        avatar_url: m.avatar_url,
        last_active: m.last_active,
        locale: m.locale,
    }
}

/// The workspace's members (answered by the platform).
pub struct Members<'a> {
    ctx: &'a Ctx,
}

impl Members<'_> {
    /// Every member (pages through the directory).
    pub async fn list(&self) -> Result<Vec<User>> {
        let mut out = Vec::new();
        let mut page_token = String::new();
        loop {
            let call = syscall::Call::MembersList(pb::MembersList {
                page_token,
                limit: 500,
            });
            match self.ctx.inner.link.syscall(call).await? {
                Some(syscall_result::Result::MembersPage(p)) => {
                    out.extend(p.members.into_iter().map(user_of));
                    if p.next_page_token.is_empty() {
                        return Ok(out);
                    }
                    page_token = p.next_page_token;
                }
                _ => {
                    return Err(Error::internal(
                        "the platform answered members_list with something else",
                    ));
                }
            }
        }
    }

    /// One member, `None` when the id is not a member.
    pub async fn get(&self, id: &Id<User>) -> Result<Option<User>> {
        let call = syscall::Call::MemberGet(pb::MemberGet {
            user_id: id.key().clone(),
        });
        match self.ctx.inner.link.syscall(call).await {
            Ok(Some(syscall_result::Result::Member(m))) => Ok(Some(user_of(m))),
            Ok(_) => Err(Error::internal(
                "the platform answered member_get with something else",
            )),
            Err(e) if e.status() == 404 => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// The workspace's groups.
pub struct Groups<'a> {
    ctx: &'a Ctx,
}

impl Groups<'_> {
    pub async fn list(&self) -> Result<Vec<Group>> {
        let mut out = Vec::new();
        let mut page_token = String::new();
        loop {
            let call = syscall::Call::GroupsList(pb::GroupsList {
                page_token,
                limit: 500,
            });
            match self.ctx.inner.link.syscall(call).await? {
                Some(syscall_result::Result::GroupsPage(p)) => {
                    out.extend(p.groups.into_iter().map(|g| Group {
                        id: Id::new(g.id),
                        name: g.name,
                        members: g.members.into_iter().map(Id::new).collect(),
                    }));
                    if p.next_page_token.is_empty() {
                        return Ok(out);
                    }
                    page_token = p.next_page_token;
                }
                _ => {
                    return Err(Error::internal(
                        "the platform answered groups_list with something else",
                    ));
                }
            }
        }
    }
}

/// Events: not available, `emit` answers `unavailable`.
pub struct Events {
    _private: (),
}

impl Events {
    pub async fn emit<M: Message>(&self, event: M) -> Result<()> {
        drop(event);
        Err(Error::unavailable(EVENTS_UNAVAILABLE))
    }
}

#[cfg(test)]
mod denial_tests {
    #[test]
    fn admission_refusals_are_403_with_their_code() {
        let e = super::denied(
            "call_undeclared: the calling plugin does not declare /c.v1.C/D among its calls",
        )
        .unwrap();
        assert_eq!((e.status(), e.code()), (403, "call_undeclared"));
        assert!(super::denied("membership_stale: the member table is not current").is_none());
        assert!(super::denied("not found").is_none());
    }

    #[test]
    fn db_refusals_keep_their_code_and_details() {
        let details = celobus_proto::DbErrorDetails {
            current_version: 7,
            column: "number".into(),
        };
        let e = super::db_refusal("version_conflict: at version 7", Some(&details)).unwrap();
        assert_eq!(
            (e.status(), e.code(), e.message()),
            (409, "version_conflict", "at version 7")
        );
        assert_eq!(e.details()["current"], 7);
        let e = super::db_refusal("unique_conflict: taken", Some(&details)).unwrap();
        assert_eq!(e.details()["column"], "number");
        for (error, status) in [
            ("not_found: no note 1", 404),
            ("query_rows_limit: more", 422),
            ("bulk_limit: more", 422),
            ("transaction_timeout: late", 504),
            ("transaction_conflict: retry", 409),
            ("invalid: bad", 400),
            ("permission_denied: not yours", 403),
            ("guard_denied: not so", 403),
            ("capability_missing: no bypass", 403),
        ] {
            assert_eq!(
                super::db_refusal(error, None).unwrap().status(),
                status,
                "{error}"
            );
        }
        assert!(super::db_refusal("unavailable: down", None).is_none());
    }

    #[test]
    fn agent_refusals_keep_the_platforms_code() {
        for (error, status, code) in [
            (
                "unknown_agent: the plugin shop declares no agent x",
                404,
                "unknown_agent",
            ),
            ("not_found: no such run", 404, "not_found"),
            ("run_ended: the run has ended", 409, "run_ended"),
            (
                "unknown_group: the platform remembers no group run-1",
                404,
                "unknown_group",
            ),
            (
                "agent_in_group: shop.helper works in this group already",
                409,
                "agent_in_group",
            ),
            ("group_full: the group has 8 live runs", 409, "group_full"),
            ("bad_request: input is empty", 400, "bad_request"),
            ("forbidden: not a member", 403, "forbidden"),
        ] {
            let e = super::denied(error).unwrap();
            assert_eq!((e.status(), e.code()), (status, code));
        }
        assert_eq!(
            super::denied("run_ended: the run has ended")
                .unwrap()
                .message(),
            "the run has ended"
        );
        assert!(super::denied("store: the store is away").is_none());
    }
}

#[cfg(test)]
mod key_tests {
    use super::canonical_key;
    use serde_json::json;

    #[test]
    fn keys_are_snake_sorted_strings() {
        assert_eq!(
            canonical_key(&json!({"orderId": "1048", "line": 2, "open": true})),
            r#"{"line":"2","open":"true","order_id":"1048"}"#
        );
        assert_eq!(canonical_key(&json!({})), "");
    }
}
