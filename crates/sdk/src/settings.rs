//! Plugin settings: what an owner or an admin of the workspace sets for the installed plugin (a limit, a webhook, an
//! API key). The plugin declares them as one `#[message(settings)]` struct — its fields' `#[field(label = ..,
//! default = .., min = ..)]` options give the form and the validators — and reads them with `ctx.settings::<S>()`.
//!
//! ```no_run
//! use celobus_sdk::prelude::*;
//!
//! /// How the chat treats agents in threads.
//! #[derive(Message, Clone, Debug)]
//! #[message(settings)]
//! pub struct ChatSettings {
//!     /// After this many hours without messages a thread no longer sticks to the agent that answered in it.
//!     #[field(label = "Thread stays with its agent, hours", default = 2, min = 1, max = 48)]
//!     pub thread_warm_hours: i32,
//!     #[field(label = "Webhook URL", url, section = "Integrations")]
//!     pub webhook_url: Option<String>,
//!     #[field(section = "Integrations")]
//!     pub webhook_token: Option<Secret>,
//! }
//!
//! async fn check(_ctx: Ctx, change: SettingsChange<ChatSettings>) -> Result<()> {
//!     if change.current.webhook_token.is_some() && change.current.webhook_url.is_none() {
//!         return Err(Error::invalid("a token needs a URL").field("webhook_url", "Set the URL the token is for"));
//!     }
//!     Ok(())
//! }
//!
//! fn main() {
//!     Plugin::new("chat", env!("CARGO_PKG_VERSION"))
//!         .settings(Settings::<ChatSettings>::new().on_change(check))
//!         .serve()
//! }
//! ```

use std::any::{Any, TypeId};
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use serde_json::{Map, Value};

use crate::ctx::Ctx;
use crate::endpoint::Handler;
use crate::error::{Error, Result};
use crate::message::{FieldType, FieldValue, Message, Registry, TypeName, wrong_type};
use crate::ui::{FormB, doc};

/// A secret setting (an API key, a token): a string the plugin reads with [`Secret::expose`], kept encrypted by the
/// platform and never shown again once saved. Its `Debug` and `Display` print `Secret(••••)`, so it stays out of
/// logs. In JSON it is a plain string.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself: pass it where it is needed, never into a log or an answer.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(••••)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(••••)")
    }
}

impl FieldValue for Secret {
    fn field_type(_: &mut Registry) -> FieldType {
        FieldType::scalar("secret")
    }
    fn encode(&self) -> Value {
        Value::String(self.0.clone())
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        value
            .as_str()
            .map(|s| Self(s.to_string()))
            .ok_or_else(|| wrong_type(path, "a string"))
    }
}

impl crate::query::Nullable for Secret {
    type Opt = Option<Secret>;
}

/// A change of the settings the plugin checks before it is saved ([`Settings::on_change`]).
#[derive(Debug, Clone)]
pub struct SettingsChange<S> {
    /// The settings before the change; `None` when they did not read as `S` (a required value was never set).
    pub previous: Option<S>,
    /// The settings to be saved, as `ctx.settings::<S>()` would read them.
    pub current: S,
}

/// The plugin's settings: `Plugin::settings(Settings::<ChatSettings>::new())`, with an optional check of each change
/// ([`Settings::on_change`]) and an optional form laid out by hand ([`Settings::form`]). Without one the form follows
/// the struct: its fields in order, in sections by their `section`; an enum is a select, a list of enum values a
/// multi-select, `bool` a switch, a number a number, a `multiline` string a text area, a list of strings a text area
/// (one per line), a `Secret` a secret input, an `Id<User>` a member.
pub struct Settings<S> {
    pub(crate) on_change: Option<Handler>,
    pub(crate) form: Option<Vec<doc::FormSection>>,
    _s: PhantomData<fn() -> S>,
}

impl<S: Message> Default for Settings<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Message> Settings<S> {
    pub fn new() -> Self {
        Self {
            on_change: None,
            form: None,
            _s: PhantomData,
        }
    }

    /// Checks a change before it is saved, for what validators cannot say (fields that go together, an outside service
    /// that must answer). The handler runs as a system request (no caller; it reads the plugin's own data and calls out)
    /// within 10 seconds. `Ok` lets the change be saved. An error refuses it: `Error::invalid(..).field("webhook_url",
    /// "Didn't answer")` shows each message under its field, any other error its message.
    pub fn on_change<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(Ctx, SettingsChange<S>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        self.on_change = Some(Arc::new(move |ctx: Ctx, body: Value| {
            let handler = handler.clone();
            Box::pin(async move {
                let current = read::<S>(body.get("current").unwrap_or(&Value::Null))?;
                let previous = body
                    .get("previous")
                    .filter(|v| v.is_object())
                    .and_then(|v| read::<S>(v).ok());
                handler(ctx, SettingsChange { previous, current }).await?;
                Ok(Value::Object(Map::new()))
            })
        }));
        self
    }

    /// Lays the form out by hand: `.form(|f| { f.section("Agents"); f.number(ChatSettings::thread_warm_hours,
    /// "Hours"); .. })`, every field once. Each input still gets its field's help (the doc comment), placeholder,
    /// whether it is required and an enum's options, unless it says its own.
    pub fn form(mut self, build: impl FnOnce(&mut FormB<S>)) -> Self {
        let mut f = FormB::<S>::settings();
        build(&mut f);
        self.form = Some(f.into_sections());
        self
    }
}

/// The settings as `Plugin::settings` declared them.
pub(crate) struct SettingsDecl {
    pub(crate) message: &'static str,
    pub(crate) register: fn(&mut Registry),
    pub(crate) on_change: Option<Handler>,
    pub(crate) form: Option<Vec<doc::FormSection>>,
}

impl<S: Message> From<Settings<S>> for SettingsDecl {
    fn from(s: Settings<S>) -> Self {
        Self {
            message: S::NAME,
            register: S::register,
            on_change: s.on_change,
            form: s.form,
        }
    }
}

/// Reads settings JSON (the plugin's JSON mapping) as `S`: the declared defaults fill what is absent; a required field
/// with no value is a 409 `settings_required` naming it.
fn read<S: Message>(json: &Value) -> Result<S> {
    let mut reg = Registry::default();
    S::register(&mut reg);
    let Some(m) = reg.messages.iter().find(|m| m.name == S::NAME) else {
        return Err(Error::internal(format!(
            "{} is not a message: declare it with #[derive(Message)] and #[message(settings)]",
            S::NAME
        )));
    };
    if m.kind != "settings" {
        return Err(Error::internal(format!(
            "{} is not a settings message: declare it with #[message(settings)]",
            S::NAME
        )));
    }
    let mut obj = match json {
        Value::Object(o) => o.clone(),
        Value::Null => Map::new(),
        _ => return Err(Error::internal("the settings are not a JSON object")),
    };
    let mut missing = vec![];
    for f in &m.fields {
        let json_name = celobus_descriptor::settings::json_name(f.name);
        let present = [json_name.as_str(), f.name]
            .iter()
            .any(|k| obj.get(*k).is_some_and(|v| !v.is_null()));
        if present {
            continue;
        }
        match f.settings.iter().find(|(k, _)| *k == "default") {
            Some((_, text)) => {
                let v: Value = serde_json::from_str(text).map_err(Error::internal)?;
                let v = match (&f.ty.name, v) {
                    (TypeName::Scalar("int64"), Value::Number(n)) if !f.ty.repeated => {
                        Value::String(n.to_string())
                    }
                    (_, v) => v,
                };
                obj.insert(json_name, v);
            }
            None if !f.ty.optional && !f.ty.repeated => missing.push(f.name),
            None => {}
        }
    }
    if !missing.is_empty() {
        let mut err = Error::new(
            409,
            "settings_required",
            format!(
                "The plugin needs setup: an owner or an admin sets {} in its settings",
                missing.join(", ")
            ),
        );
        for f in &missing {
            err = err.field(*f, "Required");
        }
        return Err(err.detail("missing", missing));
    }
    S::from_json(&Value::Object(obj), "").map_err(|e| {
        Error::internal(format!(
            "the settings do not read as {}: {}",
            S::NAME,
            e.message()
        ))
    })
}

/// The settings a process holds: the latest JSON the platform sent (or a test gave), and the value last read from it.
#[derive(Default)]
pub(crate) struct SettingsCache {
    /// the settings message the plugin declares (`None`: none, or a test context)
    declared: Option<&'static str>,
    state: RwLock<State>,
}

#[derive(Default)]
struct State {
    rev: u64,
    json: Option<Arc<Value>>,
    read: Option<(TypeId, Arc<dyn Any + Send + Sync>)>,
}

impl fmt::Debug for SettingsCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rev = self.state.read().map(|s| s.rev).unwrap_or_default();
        f.debug_struct("SettingsCache")
            .field("declared", &self.declared)
            .field("rev", &rev)
            .finish()
    }
}

impl SettingsCache {
    pub(crate) fn declared(message: Option<&'static str>) -> Self {
        Self {
            declared: message,
            state: Default::default(),
        }
    }

    /// Takes settings from the platform: at registration always, after it only a revision not older than the held one.
    /// Empty bytes are no settings. Whether they were taken.
    pub(crate) fn update(&self, rev: u64, json: &[u8], registered: bool) -> bool {
        let value = if json.iter().all(u8::is_ascii_whitespace) {
            None
        } else {
            match serde_json::from_slice::<Value>(json) {
                Ok(v) => Some(Arc::new(v)),
                Err(e) => {
                    tracing::warn!(error = %e, "the settings the platform sent are not JSON; kept the previous ones");
                    return false;
                }
            }
        };
        let mut st = self.state.write().expect("settings");
        if !registered && rev < st.rev {
            return false;
        }
        st.rev = rev;
        st.json = value;
        st.read = None;
        true
    }

    /// Settings a test gives (`Ctx::with_settings`).
    pub(crate) fn with(json: Value) -> Self {
        Self {
            declared: None,
            state: RwLock::new(State {
                rev: 0,
                json: Some(Arc::new(json)),
                read: None,
            }),
        }
    }

    pub(crate) fn get<S: Message>(&self) -> Result<Arc<S>> {
        if let Some(declared) = self.declared
            && declared != S::NAME
        {
            return Err(Error::internal(format!(
                "{} is not the plugin's settings: Plugin::settings declares {declared}",
                S::NAME
            )));
        }
        let (rev, json) = {
            let st = self.state.read().expect("settings");
            if let Some((ty, value)) = &st.read
                && *ty == TypeId::of::<S>()
                && let Ok(s) = value.clone().downcast::<S>()
            {
                return Ok(s);
            }
            (st.rev, st.json.clone())
        };
        // a platform that sent no settings: the declared defaults
        let json = match (json, self.declared) {
            (Some(json), _) => json,
            (None, Some(_)) => Arc::new(Value::Object(Map::new())),
            (None, None) => {
                return Err(Error::internal(format!(
                    "the plugin declares no settings: Plugin::settings(Settings::<{}>::new()) (in a test, \
                     Ctx::test(..).with_settings(..))",
                    S::NAME
                )));
            }
        };
        let s = Arc::new(read::<S>(&json)?);
        let mut st = self.state.write().expect("settings");
        if st.rev == rev && st.json.as_ref().is_some_and(|j| Arc::ptr_eq(j, &json)) {
            st.read = Some((TypeId::of::<S>(), s.clone()));
        }
        Ok(s)
    }
}

impl Ctx {
    /// The plugin's settings (`Plugin::settings`): what an owner or an admin saved, else the declared defaults. Read
    /// from the process's memory (the platform sends each change), so it is cheap to call in every handler. A
    /// required setting no one has set yet is a 409 `settings_required` naming it: the plugin needs setup.
    pub fn settings<S: Message>(&self) -> Result<Arc<S>> {
        self.plugin().settings_cache().get::<S>()
    }
}
