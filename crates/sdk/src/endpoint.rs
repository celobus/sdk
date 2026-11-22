//! Endpoints: a unary RPC of the plugin's service, named by its request message.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use serde_json::Value;

use crate::ctx::Ctx;
use crate::error::Result;
use crate::identity::{Role, Scope};
use crate::message::{Message, Registry};

/// Who may call an endpoint: a signed-in user (default), a service, either, or anyone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Auth {
    #[default]
    User,
    Service,
    Any,
    Public,
}

impl Auth {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Auth::User => "user",
            Auth::Service => "service",
            Auth::Any => "any",
            Auth::Public => "public",
        }
    }
}

/// Whether an `Idempotency-Key` is used: off (default), optional, required.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Idempotency {
    #[default]
    Off,
    Optional,
    Required,
}

impl Idempotency {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Idempotency::Off => "off",
            Idempotency::Optional => "optional",
            Idempotency::Required => "required",
        }
    }
}

pub(crate) type Handler =
    Arc<dyn Fn(Ctx, Value) -> BoxFuture<'static, Result<Value>> + Send + Sync>;

/// An endpoint: `Endpoint::new(handler)`, where the handler is `async fn(Ctx, XRequest) -> Result<Y>`.
/// Its name is the request message's name minus `Request` (`PlaceOrderRequest` → `PlaceOrder`).
///
/// Defaults: auth `user`, no roles (any member who passes the scopes), scopes `[write]`
/// (`.read_only()`: `[read]`), idempotency off, timeout 30 s.
#[derive(Clone)]
pub struct Endpoint {
    pub(crate) input: &'static str,
    pub(crate) output: &'static str,
    pub(crate) register: fn(&mut Registry),
    pub(crate) handler: Handler,
    pub(crate) description: String,
    pub(crate) roles: Vec<Role>,
    pub(crate) scopes: Option<Vec<Scope>>,
    pub(crate) read_only: bool,
    pub(crate) auth: Auth,
    pub(crate) idempotency: Idempotency,
    pub(crate) timeout: Duration,
    pub(crate) ui: bool,
    pub(crate) invalidates: Vec<crate::ui::Invalidation>,
}

fn register_pair<Req: Message, Resp: Message>(reg: &mut Registry) {
    Req::register(reg);
    Resp::register(reg);
}

impl Endpoint {
    pub fn new<Req, Resp, F, Fut>(handler: F) -> Self
    where
        Req: Message,
        Resp: Message,
        F: Fn(Ctx, Req) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Resp>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        let handler: Handler = Arc::new(move |ctx: Ctx, body: Value| {
            let handler = handler.clone();
            Box::pin(async move {
                let req = Req::from_json(&body, "")?;
                let resp = handler(ctx, req).await?;
                Ok(resp.to_json())
            })
        });
        Self {
            input: Req::NAME,
            output: Resp::NAME,
            register: register_pair::<Req, Resp>,
            handler,
            description: String::new(),
            roles: Vec::new(),
            scopes: None,
            read_only: false,
            auth: Auth::User,
            idempotency: Idempotency::Off,
            timeout: Duration::from_secs(30),
            ui: false,
            invalidates: Vec::new(),
        }
    }

    /// Screens may call it: queries (when it is `.read_only()`) and actions. Other endpoints stay out of reach of a
    /// UI document.
    pub fn ui(mut self) -> Self {
        self.ui = true;
        self
    }

    /// After a 2xx answer the platform invalidates the topic (every key, or the key taken from the response:
    /// `ORDER.by(OrderKey::id, Order::id)`); open screens following it re-read.
    pub fn invalidates(mut self, t: impl Into<crate::ui::Invalidation>) -> Self {
        self.invalidates.push(t.into());
        self
    }

    /// Display text: what the endpoint does, for people and agents.
    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Any of these roles may call it: the plugin's own or `Role::WORKSPACE_*`.
    pub fn roles(mut self, roles: impl IntoIterator<Item = Role>) -> Self {
        self.roles = roles.into_iter().collect();
        self
    }

    /// The caller's token must carry all of these scopes (default `[write]`).
    pub fn scopes(mut self, scopes: impl IntoIterator<Item = Scope>) -> Self {
        self.scopes = Some(scopes.into_iter().collect());
        self
    }

    /// The endpoint only reads: its scope is `read` (unless `.scopes(..)` names others).
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    pub fn auth(mut self, auth: Auth) -> Self {
        self.auth = auth;
        self
    }

    pub fn idempotency(mut self, idempotency: Idempotency) -> Self {
        self.idempotency = idempotency;
        self
    }

    /// At most 600 s (default 30 s).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The endpoint's name, or `None` when the request type does not end in `Request`.
    pub fn name(&self) -> Option<&'static str> {
        self.input.strip_suffix("Request").filter(|n| !n.is_empty())
    }

    pub(crate) fn scope_names(&self) -> Vec<&'static str> {
        match &self.scopes {
            Some(scopes) => scopes.iter().map(|s| s.name()).collect(),
            None if self.read_only => vec![Scope::READ.name()],
            None => vec![Scope::WRITE.name()],
        }
    }
}
