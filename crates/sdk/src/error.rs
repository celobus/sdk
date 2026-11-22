//! Runtime errors: one `Error` type, mapped to an HTTP status and `{"code", "message", "details"}`.

use std::borrow::Cow;
use std::fmt;

use serde_json::{Map, Value};

/// The result of a handler and of every `ctx` call.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An error a handler returns: a status, a stable code, a message and details.
///
/// `invalid` 400 (with `details.fields`), `unauthenticated` 401, `forbidden` 403, `not_found` 404,
/// `conflict` 409, a domain error (`ErrorCode::error`) with its own code and status, `unavailable` 503,
/// `deadline_exceeded` 504, `internal` 500 (its message is logged, never sent). `?` on an
/// `anyhow::Error`, an I/O or a JSON error gives `internal`.
///
/// In a job's handler an error fails the attempt, and the run is retried unless the error is permanent
/// ([`Error::is_permanent`]): [`Error::permanent`], an error marked with [`Error::into_permanent`], or an input-class
/// error (`invalid`, `not_found`, `permission_denied`, `payload_invalid`).
#[derive(Debug, Clone)]
pub struct Error {
    status: u16,
    code: Cow<'static, str>,
    message: String,
    details: Map<String, Value>,
    permanent: bool,
}

/// The codes of input-class errors: a job's attempt failing with one of them is not retried.
const PERMANENT_CODES: &[&str] = &[
    "invalid",
    "not_found",
    "permission_denied",
    "payload_invalid",
];

impl Error {
    /// An error with any status and code (prefer the constructors below).
    pub fn new(
        status: u16,
        code: impl Into<Cow<'static, str>>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            details: Map::new(),
            permanent: false,
        }
    }

    /// 422 `permanent`: in a job, the attempt fails and the run is not retried (bad input that a retry cannot fix).
    pub fn permanent(message: impl Into<String>) -> Self {
        Self::new(422, "permanent", message).into_permanent()
    }

    /// The same error, marked permanent: a job's run that fails with it is not retried.
    pub fn into_permanent(mut self) -> Self {
        self.permanent = true;
        self
    }

    /// Whether a job's run that fails with this error is not retried: marked permanent, or an input-class error
    /// (`invalid`, `not_found`, `permission_denied`, `payload_invalid`).
    pub fn is_permanent(&self) -> bool {
        self.permanent || PERMANENT_CODES.contains(&self.code.as_ref())
    }

    /// 400 `invalid`; name the fields with [`Error::field`].
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(400, "invalid", message)
    }

    /// 401 `unauthenticated`.
    pub fn unauthenticated(message: impl Into<String>) -> Self {
        Self::new(401, "unauthenticated", message)
    }

    /// 403 `forbidden`.
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(403, "forbidden", message)
    }

    /// 404 `not_found`.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, "not_found", message)
    }

    /// 409 `conflict`.
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(409, "conflict", message)
    }

    /// 503 `unavailable`: the platform, or an outside service the plugin calls, did not answer.
    pub fn unavailable(message: impl fmt::Display) -> Self {
        Self::new(503, "unavailable", message.to_string())
    }

    /// 504 `deadline_exceeded`.
    pub fn deadline_exceeded(message: impl Into<String>) -> Self {
        Self::new(504, "deadline_exceeded", message)
    }

    /// 500 `internal`: the message is logged, the caller gets "internal error".
    pub fn internal(message: impl fmt::Display) -> Self {
        Self::new(500, "internal", message.to_string())
    }

    /// Adds `details.fields[path] = message` (an `invalid` error names the fields it is about).
    pub fn field(mut self, path: impl Into<String>, message: impl Into<String>) -> Self {
        let fields = self
            .details
            .entry("fields")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(fields) = fields {
            fields.insert(path.into(), Value::String(message.into()));
        }
        self
    }

    /// Adds a detail (`details[key] = value`).
    pub fn detail(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }

    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn details(&self) -> &Map<String, Value> {
        &self.details
    }

    /// The `invalid` error's field messages (`details.fields`).
    pub fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.details
            .get("fields")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k.as_str(), v.as_str()?)))
    }

    /// The wire body: `{"code", "message", "details"}`; an internal error's message is hidden.
    pub fn body(&self) -> Value {
        let message = if self.status >= 500 && self.code == "internal" {
            "internal error".to_string()
        } else {
            self.message.clone()
        };
        serde_json::json!({
            "code": self.code,
            "message": message,
            "details": Value::Object(self.details.clone()),
        })
    }

    /// An error answer of another plugin (`ctx.call`): its status and its `{"code", "message",
    /// "details"}` body, kept as they are.
    pub(crate) fn from_answer(status: u16, body: &[u8]) -> Self {
        let parsed: Option<Map<String, Value>> = serde_json::from_slice(body).ok();
        match parsed {
            Some(obj) if obj.get("code").and_then(Value::as_str).is_some() => {
                let code = obj["code"].as_str().unwrap_or_default().to_string();
                let message = obj
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let mut err = Self::new(status, code, message);
                if let Some(Value::Object(details)) = obj.get("details") {
                    err.details = details.clone();
                }
                err
            }
            _ => Self::new(
                status,
                default_code(status),
                String::from_utf8_lossy(body)
                    .chars()
                    .take(500)
                    .collect::<String>(),
            ),
        }
    }
}

fn default_code(status: u16) -> &'static str {
    match status {
        400 => "invalid",
        401 => "unauthenticated",
        403 => "forbidden",
        404 => "not_found",
        409 => "conflict",
        503 => "unavailable",
        504 => "deadline_exceeded",
        _ => "internal",
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}: {}", self.status, self.code, self.message)
    }
}

impl std::error::Error for Error {}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Self::internal(format!("{e:#}"))
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::internal(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::internal(e)
    }
}

/// A domain error of the plugin: a code and the status it answers with (400–422).
///
/// ```
/// use celobus_sdk::prelude::*;
/// pub const NOT_CANCELLABLE: ErrorCode = ErrorCode::new("not_cancellable", 409).describe("The order is no longer placed.");
/// let err = NOT_CANCELLABLE.error("order SO-1 is shipped");
/// assert_eq!((err.status(), err.code()), (409, "not_cancellable"));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorCode {
    code: &'static str,
    status: u16,
    description: &'static str,
}

impl ErrorCode {
    pub const fn new(code: &'static str, status: u16) -> Self {
        Self {
            code,
            status,
            description: "",
        }
    }

    /// 400.
    pub const fn invalid(code: &'static str) -> Self {
        Self::new(code, 400)
    }

    /// 403.
    pub const fn forbidden(code: &'static str) -> Self {
        Self::new(code, 403)
    }

    /// 404.
    pub const fn not_found(code: &'static str) -> Self {
        Self::new(code, 404)
    }

    /// 409.
    pub const fn conflict(code: &'static str) -> Self {
        Self::new(code, 409)
    }

    /// 422.
    pub const fn unprocessable(code: &'static str) -> Self {
        Self::new(code, 422)
    }

    pub const fn describe(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    pub const fn code(&self) -> &'static str {
        self.code
    }

    pub const fn status(&self) -> u16 {
        self.status
    }

    pub const fn description(&self) -> &'static str {
        self.description
    }

    /// The error, with this code and status.
    pub fn error(&self, message: impl Into<String>) -> Error {
        Error::new(self.status, self.code, message)
    }
}
