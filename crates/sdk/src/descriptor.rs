//! The plugin descriptor (schema 1): what `--describe` prints.
//!
//! Every field of the schema is written, with its default when not declared, so the three SDKs emit
//! the same shape. Arrays keep the declaration order; `messages` and `enums` are in the order of first
//! use while walking the endpoints as declared (input, then output), then the entities, then the extra
//! messages, each message before the messages and enums its fields use (depth first, in field order).
//! An entity's message is listed in `messages` too.

use std::fmt;

use serde::Serialize;
use serde_json::Value;

/// A definition error the SDK can see before serving: `error[E_CODE] at <path>: <message>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribeError {
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

impl DescribeError {
    pub(crate) fn new(
        code: &'static str,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for DescribeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error[{}] at {}: {}", self.code, self.path, self.message)
    }
}

/// All the definition errors of a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribeErrors(pub Vec<DescribeError>);

impl fmt::Display for DescribeErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for DescribeErrors {}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Descriptor {
    pub schema: u32,
    pub id: String,
    pub version: String,
    pub title: String,
    pub description: String,
    pub roles: Vec<RoleDecl>,
    pub scopes: Vec<ScopeDecl>,
    pub capabilities: Vec<CapabilityDecl>,
    pub permissions: Vec<String>,
    pub agents: Vec<AgentDecl>,
    /// Written only when declared.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ConnectionDecl>,
    pub dependencies: Vec<DependencyDecl>,
    pub enums: Vec<EnumDecl>,
    pub messages: Vec<MessageDecl>,
    pub entities: Vec<EntityDecl>,
    pub endpoints: Vec<EndpointDecl>,
    /// Written only when declared, so plugins without them describe as before.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub topics: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui: Option<Value>,
    /// The notification kinds; written only when declared.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notifications: Vec<NotificationDecl>,
    /// What the plugin asks of agent runs; written only when it handles the run-change hook.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runs: Option<RunsDecl>,
    /// The background jobs; written only when declared.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub jobs: Vec<celobus_descriptor::Job>,
}

/// `runs`: the plugin handles the run-change hook (`on_change`).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RunsDecl {
    pub on_change: bool,
}

/// A notification kind: every field written, as in the other SDKs.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NotificationDecl {
    pub name: String,
    pub title: String,
    pub description: String,
    pub tab: String,
    pub digest: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RoleDecl {
    pub name: String,
    pub title: String,
    pub description: String,
    pub strict: bool,
    pub default_for: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ScopeDecl {
    pub name: String,
    pub title: String,
    pub description: String,
    pub sensitive: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CapabilityDecl {
    pub name: String,
    pub reason: String,
    /// `authz:bypass` only: entities and operations; written only when declared.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub on: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentDecl {
    pub name: String,
    pub roles: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub workspace: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub delegates: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub connection: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub ssh_key: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub admin: bool,
    // how the agent runs: each written only when declared, so an agent without them describes as before
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub icon: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub engine: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub session: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_s: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub topic: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConnectionDecl {
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DependencyDecl {
    pub plugin: String,
    pub calls: Vec<String>,
    /// Written only when declared.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<ReadDecl>,
}

/// An entity of a dependency the plugin reads, with the fields it reads.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReadDecl {
    pub entity: String,
    pub fields: Vec<ReadFieldDecl>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReadFieldDecl {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EnumDecl {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MessageDecl {
    pub name: String,
    pub kind: String,
    pub description: String,
    pub fields: Vec<FieldDecl>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EntityDecl {
    pub name: String,
    pub message: String,
    pub key: String,
    pub label: String,
    pub soft_delete: bool,
    pub version: bool,
    pub append_only: bool,
    pub fields: Vec<FieldDecl>,
    /// Written only when declared, so entities without them describe as before.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub indexes: Vec<IndexDecl>,
    /// The access rules; written only when declared (none: no one but a system context).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access: Option<Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IndexDecl {
    pub columns: Vec<IndexColumnDecl>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unique: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IndexColumnDecl {
    pub name: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub desc: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FieldDecl {
    pub name: String,
    pub number: u32,
    #[serde(rename = "type")]
    pub ty: String,
    pub repeated: bool,
    pub optional: bool,
    pub description: String,
    /// Entity columns only, each written only when declared.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unique: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub index: bool,
    /// Kept encrypted at rest with the workspace's key.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub encrypted: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub searchable: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub nav: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub inverse: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub on_delete: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EndpointDecl {
    pub name: String,
    pub input: String,
    pub output: String,
    pub description: String,
    pub auth: String,
    pub roles: Vec<String>,
    pub scopes: Vec<String>,
    pub idempotency: String,
    pub timeout_ms: u64,
    /// `ui` endpoints only (the field matters to UI queries; other endpoints describe as before).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub ui: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub invalidates: Vec<Value>,
}

impl Descriptor {
    /// The canonical JSON: object keys sorted, arrays as declared, two-space indentation, a final
    /// newline (what `jq -S .` prints).
    pub fn to_canonical_json(&self) -> String {
        let value = serde_json::to_value(self).expect("the descriptor serializes");
        let mut out = String::new();
        write_canonical(&value, 0, &mut out);
        out.push('\n');
        out
    }
}

fn write_canonical(value: &Value, indent: usize, out: &mut String) {
    let pad = |n: usize, out: &mut String| out.extend(std::iter::repeat_n(' ', n));
    match value {
        Value::Object(map) if !map.is_empty() => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push_str("{\n");
            for (i, k) in keys.iter().enumerate() {
                pad(indent + 2, out);
                out.push_str(&Value::String((*k).clone()).to_string());
                out.push_str(": ");
                write_canonical(&map[*k], indent + 2, out);
                if i + 1 < keys.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            pad(indent, out);
            out.push('}');
        }
        Value::Array(items) if !items.is_empty() => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                pad(indent + 2, out);
                write_canonical(item, indent + 2, out);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            pad(indent, out);
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}
