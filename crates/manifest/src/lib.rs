//! Plugin manifest: what a plugin declares about itself (roles, scopes, agents, dependencies, permissions,
//! method requirements). It is installed into a workspace with the plugin and checked by the platform
//! against the running plugin (`manifest.json` next to the plugin, `Register.manifest_digest`).
//!
//! JSON form (`manifest.json`):
//! ```json
//! { "roles": [{ "name": "manager", "title": "Manager", "description": "...", "default_for": "member" }],
//!   "scopes": [{ "name": "deals_read", "title": "Read deals", "sensitive": false }],
//!   "connections": [{ "name": "models", "kind": "llm", "title": "Models", "description": "..." }],
//!   "agents": [{ "name": "scorer", "roles": ["scorer"], "delegates": ["writer", "catalog.pricer"],
//!                "connection": "models", "ssh_key": true, "admin": true }],
//!   "depends": ["catalog"], "permissions": ["members.email"],
//!   "requires": [{ "method": "/crm.v1.Crm/ListDeals", "scope": "deals_read", "any_role": ["manager", "workspace.admin"] }] }
//! ```

use celobus_proto as pb;
use prost::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default)]
    pub roles: Vec<Role>,
    #[serde(default)]
    pub scopes: Vec<Scope>,
    #[serde(default)]
    pub agents: Vec<Agent>,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub requires: Vec<Requirement>,
    /// The dependency endpoints the plugin calls (`/<dep>.v1.<Service>/<Method>`): an invoke of another endpoint
    /// of a dependency listed here is refused.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<String>,
    /// The connections the plugin declares: installing it creates them, not set up, in the
    /// workspace. Written only when declared, so a plugin without them keeps its manifest and its digest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<Connection>,
}

/// The kinds of a connection: a language model provider for agents, speech recognition for dictation, a basemap
/// provider for the plugin's maps.
pub const CONNECTION_KINDS: &[&str] = &["llm", "stt", "maps"];

/// A connection a plugin declares; an owner or admin of the workspace fills it in.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    /// snake_case, at most 32 characters, unique in the plugin
    pub name: String,
    /// one of [`CONNECTION_KINDS`]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Role {
    pub name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// none | member | admin
    #[serde(default)]
    pub default_for: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// The scope reaches sensitive data or acts on someone's behalf: install screens call it out. Not part of
    /// the protobuf form, so it never changes the digest.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub name: String,
    #[serde(default)]
    pub roles: Vec<String>,
    /// The workspace permissions the agent asks for (`members.write`, `plugins.install`, …): with one, it does the
    /// workspace operations that take it as a workspace admin would. Whoever installs the plugin sees them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace: Vec<String>,
    /// The agents this one may bring into its group when they are not there: an agent of this plugin by its
    /// name, or `<dependency>.<agent>`. Whoever installs the plugin sees them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delegates: Vec<String>,
    /// The `llm` connection its runs use: `<name>` of a connection of this plugin, or `<dependency>.<name>`.
    /// Empty: none. Whoever installs the plugin sees it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub connection: String,
    /// The agent has an SSH key of its own, given to its runs. Whoever installs the plugin sees it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ssh_key: bool,
    /// The agent's service account asks for base role admin instead of member: only an owner installs or
    /// upgrades to a version that asks for it, and whoever installs the plugin sees it. Written only when declared.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub admin: bool,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub method: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub any_role: Vec<String>,
}

/// Names a plugin may never take: the platform's own scope areas (`core:write`, `system:admin`, ...), other
/// reserved ids and the platform's own words (`api`, `app`, `sdk`, `cell`, ...). Plugin scopes under them are
/// refused, so a plugin scope can never stand in for a platform scope.
pub const RESERVED_PLUGIN_NAMES: &[&str] = &[
    "core",
    "system",
    "workspace",
    "mcp",
    "lake",
    "olap",
    "dev",
    "registry",
    "realtime",
    "graphql",
    "platform",
    "admin",
    "push",
    "celobus",
    "ui",
    "placement",
    "organization",
    "id",
    "plugin",
    "sys",
    "api",
    "app",
    "auth",
    "sdk",
    "cell",
    "public",
    "internal",
];

/// Every plugin name starting with one of these is reserved too (`pg`: the database's own namespace).
pub const RESERVED_PLUGIN_PREFIXES: &[&str] = &["pg"];

/// A role or scope name: `[a-z][a-z0-9_-]{0,62}`, the grammar of the part after `<plugin>:`.
fn name_ok(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-'))
}

/// A connection name: snake_case, `[a-z][a-z0-9_]{0,31}`.
pub fn connection_name_ok(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

/// The shape of a workspace permission: `<area>.<verb>`, lowercase words (`service_accounts.write`). Which ones
/// exist is the platform's list, checked where a manifest is installed.
pub fn workspace_permission_ok(s: &str) -> bool {
    let word = |w: &str| {
        !w.is_empty()
            && w.len() <= 32
            && w.starts_with(|c: char| c.is_ascii_lowercase())
            && w.chars().all(|c| c.is_ascii_lowercase() || c == '_')
    };
    matches!(s.split_once('.'), Some((area, verb)) if word(area) && word(verb))
}

/// The plugin-name grammar alone: `[a-z][a-z0-9]{1,15}`.
pub fn plugin_name_shape_ok(s: &str) -> bool {
    let b = s.as_bytes();
    (2..=16).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// A reserved plugin name: one of [`RESERVED_PLUGIN_NAMES`] or starting with a [`RESERVED_PLUGIN_PREFIXES`] entry.
pub fn plugin_name_reserved(s: &str) -> bool {
    RESERVED_PLUGIN_NAMES.contains(&s) || RESERVED_PLUGIN_PREFIXES.iter().any(|p| s.starts_with(p))
}

/// A plugin name: `[a-z][a-z0-9]{1,15}` and not reserved, so `<plugin>:<scope>` is a valid
/// plugin scope.
pub fn plugin_name_ok(s: &str) -> bool {
    plugin_name_shape_ok(s) && !plugin_name_reserved(s)
}

impl Manifest {
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let m: Manifest =
            serde_json::from_slice(bytes).map_err(|e| format!("invalid manifest: {e}"))?;
        m.validate()?;
        Ok(m)
    }

    pub fn validate(&self) -> Result<(), String> {
        for r in &self.roles {
            if !name_ok(&r.name) {
                return Err(format!("invalid role name {:?}", r.name));
            }
            if !matches!(
                r.default_for.as_deref(),
                None | Some("none" | "member" | "admin")
            ) {
                return Err(format!(
                    "role {}: default_for must be none, member or admin",
                    r.name
                ));
            }
        }
        for s in &self.scopes {
            if !name_ok(&s.name) {
                return Err(format!("invalid scope name {:?}", s.name));
            }
            if s.name == "use" {
                return Err("scope \"use\" is implicit: every plugin has <plugin>:use".into());
            }
        }
        for d in &self.depends {
            if !plugin_name_ok(d) {
                return Err(format!("invalid dependency name {d:?}"));
            }
        }
        for (i, c) in self.connections.iter().enumerate() {
            if !connection_name_ok(&c.name) {
                return Err(format!("invalid connection name {:?}", c.name));
            }
            if !CONNECTION_KINDS.contains(&c.kind.as_str()) {
                return Err(format!(
                    "connection {}: kind must be llm, stt or maps, not {:?}",
                    c.name, c.kind
                ));
            }
            if self.connections[..i].iter().any(|x| x.name == c.name) {
                return Err(format!("connection {} is declared twice", c.name));
            }
            if c.title.chars().count() > 80 || c.description.chars().count() > 500 {
                return Err(format!(
                    "connection {}: a title is at most 80 characters, a description 500",
                    c.name
                ));
            }
        }
        for a in &self.agents {
            if !name_ok(&a.name) {
                return Err(format!("invalid agent name {:?}", a.name));
            }
            for r in &a.roles {
                // agents get only the plugin's own roles (never workspace.* or another plugin's)
                if !self.roles.iter().any(|d| &d.name == r) {
                    return Err(format!(
                        "agent {}: role {r:?} is not declared by this plugin",
                        a.name
                    ));
                }
            }
            for (i, p) in a.workspace.iter().enumerate() {
                if !workspace_permission_ok(p) {
                    return Err(format!(
                        "agent {}: invalid workspace permission {p:?} (<area>.<verb>)",
                        a.name
                    ));
                }
                if a.workspace[..i].contains(p) {
                    return Err(format!(
                        "agent {}: workspace permission {p:?} is listed twice",
                        a.name
                    ));
                }
            }
            for (i, d) in a.delegates.iter().enumerate() {
                if let Err(why) = self.delegate_ok(a, d) {
                    return Err(format!("agent {}: delegate {d:?} {why}", a.name));
                }
                if a.delegates[..i].contains(d) {
                    return Err(format!("agent {}: delegate {d:?} is listed twice", a.name));
                }
            }
            if !a.connection.is_empty()
                && let Err(why) = self.agent_connection_ok(&a.connection)
            {
                return Err(format!(
                    "agent {}: connection {:?} {why}",
                    a.name, a.connection
                ));
            }
        }
        for q in &self.requires {
            if !q.method.starts_with('/') {
                return Err(format!(
                    "requirement method {:?} must look like /pkg.Service/Method",
                    q.method
                ));
            }
            if !q.scope.is_empty() && !self.scopes.iter().any(|s| s.name == q.scope) {
                return Err(format!(
                    "requirement {}: scope {:?} is not declared",
                    q.method, q.scope
                ));
            }
        }
        for c in &self.calls {
            let target = call_target(c);
            if target.is_none_or(|t| !self.depends.iter().any(|d| d == t)) {
                return Err(format!(
                    "call {c:?} must be /<dependency>.v1.<Service>/<Method> of a declared dependency"
                ));
            }
        }
        Ok(())
    }

    /// A delegate of `agent`: another agent of this plugin by its name, or `<dependency>.<agent>` of a declared
    /// dependency. The error is what is wrong with it, to follow its name.
    pub fn delegate_ok(&self, agent: &Agent, delegate: &str) -> Result<(), &'static str> {
        match delegate.split_once('.') {
            None if delegate == agent.name => Err("is the agent itself"),
            None if self.agents.iter().any(|a| a.name == delegate) => Ok(()),
            None => Err("is not an agent of this plugin"),
            Some((plugin, name)) if !name_ok(name) || !plugin_name_shape_ok(plugin) => {
                Err("is not <dependency>.<agent>")
            }
            Some((plugin, _)) if !self.depends.iter().any(|d| d == plugin) => {
                Err("names a plugin that is not a declared dependency")
            }
            Some(_) => Ok(()),
        }
    }

    /// An agent's connection: `<name>` of an `llm` connection of this plugin, or `<dependency>.<name>` of a declared
    /// dependency (whether that plugin declares it is checked where the manifest is installed). The error is what
    /// is wrong with it, to follow its name.
    pub fn agent_connection_ok(&self, connection: &str) -> Result<(), &'static str> {
        match connection.split_once('.') {
            None => match self.connections.iter().find(|c| c.name == connection) {
                None => Err("is not a connection of this plugin"),
                Some(c) if c.kind != "llm" => Err("is not an llm connection"),
                Some(_) => Ok(()),
            },
            Some((plugin, name)) if !connection_name_ok(name) || !plugin_name_shape_ok(plugin) => {
                Err("is not <dependency>.<connection>")
            }
            Some((plugin, _)) if !self.depends.iter().any(|d| d == plugin) => {
                Err("names a plugin that is not a declared dependency")
            }
            Some(_) => Ok(()),
        }
    }

    pub fn to_pb(&self) -> pb::PluginManifest {
        pb::PluginManifest {
            roles: self
                .roles
                .iter()
                .map(|r| pb::RoleDecl {
                    name: r.name.clone(),
                    title: r.title.clone(),
                    description: r.description.clone(),
                    default_for: match r.default_for.as_deref() {
                        Some("member") => pb::role_decl::Default::Member as i32,
                        Some("admin") => pb::role_decl::Default::Admin as i32,
                        _ => pb::role_decl::Default::None as i32,
                    },
                })
                .collect(),
            scopes: self
                .scopes
                .iter()
                .map(|s| pb::ScopeDecl {
                    name: s.name.clone(),
                    title: s.title.clone(),
                    description: s.description.clone(),
                })
                .collect(),
            agents: self
                .agents
                .iter()
                .map(|a| pb::AgentDecl {
                    name: a.name.clone(),
                    roles: a.roles.clone(),
                    workspace: a.workspace.clone(),
                    delegates: a.delegates.clone(),
                    connection: a.connection.clone(),
                    ssh_key: a.ssh_key,
                    admin: a.admin,
                })
                .collect(),
            depends: self.depends.clone(),
            permissions: self.permissions.clone(),
            requires: self
                .requires
                .iter()
                .map(|q| pb::MethodRequirement {
                    method: q.method.clone(),
                    scope: q.scope.clone(),
                    any_role: q.any_role.clone(),
                })
                .collect(),
            calls: self.calls.clone(),
            connections: self
                .connections
                .iter()
                .map(|c| pb::ConnectionDecl {
                    name: c.name.clone(),
                    kind: c.kind.clone(),
                    title: c.title.clone(),
                    description: c.description.clone(),
                })
                .collect(),
        }
    }

    /// sha256 of the protobuf encoding (fields in declaration order, no maps: deterministic).
    pub fn digest(&self) -> String {
        digest_pb(&self.to_pb())
    }
}

/// The plugin a call names: `/catalog.v1.Catalog/GetProduct` → `catalog`.
pub fn call_target(method: &str) -> Option<&str> {
    let rest = method.strip_prefix('/')?;
    let (pkg, _) = rest.split_once('/')?;
    pkg.split('.').next().filter(|p| !p.is_empty())
}

pub fn digest_pb(m: &pb::PluginManifest) -> String {
    hex::encode(Sha256::digest(m.encode_to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_idp_grammar() {
        assert!(plugin_name_ok("crm"));
        assert!(plugin_name_ok("catalog2"));
        assert!(!plugin_name_ok("c"));
        assert!(!plugin_name_ok("core"));
        assert!(!plugin_name_ok("my-crm"));
        assert!(!plugin_name_ok("averyveryverylongname"));
        for reserved in [
            "api", "app", "auth", "sdk", "cell", "public", "internal", "pg", "pgcrypto",
        ] {
            assert!(!plugin_name_ok(reserved), "{reserved}");
        }
        for name in [
            "catalog",
            "customers",
            "orders",
            "billing",
            "storefront",
            "crm",
            "page",
        ] {
            assert!(plugin_name_ok(name), "{name}");
        }
        assert!(Manifest::from_json(br#"{"scopes":[{"name":"deals.read"}]}"#).is_err());
        assert!(Manifest::from_json(br#"{"scopes":[{"name":"use"}]}"#).is_err());
        assert!(Manifest::from_json(br#"{"depends":["core"]}"#).is_err());
    }

    #[test]
    fn parse_and_digest() {
        let m = Manifest::from_json(br#"{"roles":[{"name":"manager","default_for":"admin"}],
            "scopes":[{"name":"deals_read"}],
            "requires":[{"method":"/crm.v1.Crm/ListDeals","scope":"deals_read","any_role":["manager"]}]}"#)
        .unwrap();
        assert_eq!(m.digest(), m.clone().digest());
        assert_eq!(
            m.to_pb().roles[0].default_for,
            pb::role_decl::Default::Admin as i32
        );
    }

    #[test]
    fn sensitive_is_optional_and_outside_the_digest() {
        let plain = Manifest::from_json(br#"{"scopes":[{"name":"deals_read"}]}"#).unwrap();
        let marked =
            Manifest::from_json(br#"{"scopes":[{"name":"deals_read","sensitive":true}]}"#).unwrap();
        assert!(marked.scopes[0].sensitive && !plain.scopes[0].sensitive);
        assert_eq!(plain.digest(), marked.digest());
        // an unmarked scope serializes without the field
        assert!(!serde_json::to_string(&plain).unwrap().contains("sensitive"));
    }

    #[test]
    fn agents_ask_for_well_formed_workspace_permissions_once() {
        let m = Manifest::from_json(
            br#"{"agents":[{"name":"a","workspace":["plugins.install","service_accounts.write"]}]}"#,
        )
        .unwrap();
        assert_eq!(
            m.agents[0].workspace,
            ["plugins.install", "service_accounts.write"]
        );
        assert_eq!(m.to_pb().agents[0].workspace, m.agents[0].workspace);
        // an agent without permissions keeps the manifest, and so its digest, as it was
        let bare = Manifest::from_json(br#"{"agents":[{"name":"a"}]}"#).unwrap();
        assert_eq!(
            serde_json::to_string(&bare.agents[0]).unwrap(),
            r#"{"name":"a","roles":[]}"#
        );
        for bad in [
            "admin",
            "Members.write",
            "members.",
            "members.write.all",
            "*.*",
        ] {
            let json = format!(r#"{{"agents":[{{"name":"a","workspace":["{bad}"]}}]}}"#);
            assert!(Manifest::from_json(json.as_bytes()).is_err(), "{bad}");
        }
        let twice = br#"{"agents":[{"name":"a","workspace":["audit.read","audit.read"]}]}"#;
        assert!(Manifest::from_json(twice).unwrap_err().contains("twice"));
    }

    #[test]
    fn agents_delegate_to_agents_of_the_plugin_and_of_its_dependencies() {
        let m = Manifest::from_json(
            br#"{"depends":["crm"],"agents":[{"name":"a","delegates":["b","crm.scout"]},{"name":"b"}]}"#,
        )
        .unwrap();
        assert_eq!(m.agents[0].delegates, ["b", "crm.scout"]);
        assert_eq!(m.to_pb().agents[0].delegates, m.agents[0].delegates);
        // an agent without delegates keeps the manifest, and so its digest, as it was
        let bare = Manifest::from_json(br#"{"agents":[{"name":"a"}]}"#).unwrap();
        let none = Manifest::from_json(br#"{"agents":[{"name":"a","delegates":[]}]}"#).unwrap();
        assert_eq!(bare.digest(), none.digest());
        assert_eq!(
            serde_json::to_string(&none.agents[0]).unwrap(),
            r#"{"name":"a","roles":[]}"#
        );
        // the list is what whoever installs the plugin reviews: it is part of the digest
        assert_ne!(
            m.digest(),
            Manifest::from_json(br#"{"depends":["crm"],"agents":[{"name":"a"},{"name":"b"}]}"#)
                .unwrap()
                .digest()
        );
        for (delegates, why) in [
            (r#"["a"]"#, "itself"),
            (r#"["ghost"]"#, "not an agent of this plugin"),
            (r#"["shop.helper"]"#, "not a declared dependency"),
            (r#"["crm."]"#, "<dependency>.<agent>"),
            (r#"["crm.Scout"]"#, "<dependency>.<agent>"),
            (r#"["b","b"]"#, "twice"),
        ] {
            let json = format!(
                r#"{{"depends":["crm"],"agents":[{{"name":"a","delegates":{delegates}}},{{"name":"b"}}]}}"#
            );
            let err = Manifest::from_json(json.as_bytes()).unwrap_err();
            assert!(err.contains(why), "{delegates}: {err}");
        }
    }

    #[test]
    fn connections_are_declared_once_and_named_by_agents() {
        let m = Manifest::from_json(
            br#"{"depends":["ai"],
                "connections":[{"name":"research","kind":"llm","title":"Research model"},{"name":"speech","kind":"stt"},
                               {"name":"basemap","kind":"maps","title":"Maps"}],
                "agents":[{"name":"a","connection":"research","ssh_key":true},{"name":"b","connection":"ai.anthropic"}]}"#,
        )
        .unwrap();
        let pb = m.to_pb();
        assert_eq!(pb.connections[1].kind, "stt");
        assert_eq!(pb.connections[2].kind, "maps");
        assert_eq!(
            (pb.agents[0].connection.as_str(), pb.agents[0].ssh_key),
            ("research", true)
        );
        // the connections and an agent's connection and key are what whoever installs the plugin reviews
        let bare = Manifest::from_json(br#"{"depends":["ai"],"connections":[{"name":"research","kind":"llm"}],"agents":[{"name":"a"}]}"#).unwrap();
        let keyed = Manifest::from_json(br#"{"depends":["ai"],"connections":[{"name":"research","kind":"llm"}],"agents":[{"name":"a","ssh_key":true}]}"#).unwrap();
        assert_ne!(bare.digest(), keyed.digest());
        // a plugin without connections keeps its manifest, and so its digest, as it was
        let none = Manifest::from_json(br#"{"agents":[{"name":"a"}]}"#).unwrap();
        assert_eq!(
            serde_json::to_string(&none).unwrap(),
            r#"{"roles":[],"scopes":[],"agents":[{"name":"a","roles":[]}],"depends":[],"permissions":[],"requires":[]}"#
        );
        for (json, why) in [
            (
                r#"{"connections":[{"name":"Research","kind":"llm"}]}"#,
                "invalid connection name",
            ),
            (
                r#"{"connections":[{"name":"r","kind":"tts"}]}"#,
                "llm, stt or maps",
            ),
            (
                r#"{"connections":[{"name":"r","kind":"llm"},{"name":"r","kind":"stt"}]}"#,
                "twice",
            ),
            (
                r#"{"connections":[{"name":"s","kind":"stt"}],"agents":[{"name":"a","connection":"s"}]}"#,
                "not an llm",
            ),
            (
                r#"{"connections":[{"name":"m","kind":"maps"}],"agents":[{"name":"a","connection":"m"}]}"#,
                "not an llm",
            ),
            (
                r#"{"agents":[{"name":"a","connection":"ghost"}]}"#,
                "not a connection of this plugin",
            ),
            (
                r#"{"agents":[{"name":"a","connection":"shop.models"}]}"#,
                "not a declared dependency",
            ),
            (
                r#"{"depends":["shop"],"agents":[{"name":"a","connection":"shop.Models"}]}"#,
                "<dependency>.<connection>",
            ),
        ] {
            let err = Manifest::from_json(json.as_bytes()).unwrap_err();
            assert!(err.contains(why), "{json}: {err}");
        }
    }

    #[test]
    fn an_agent_asks_for_base_role_admin_in_the_digest() {
        let admin = Manifest::from_json(br#"{"agents":[{"name":"a","admin":true}]}"#).unwrap();
        assert!(admin.agents[0].admin && admin.to_pb().agents[0].admin);
        // an agent that does not ask keeps the manifest, and so its digest, as it was
        let bare = Manifest::from_json(br#"{"agents":[{"name":"a"}]}"#).unwrap();
        let no = Manifest::from_json(br#"{"agents":[{"name":"a","admin":false}]}"#).unwrap();
        assert_eq!(bare.digest(), no.digest());
        assert_eq!(
            serde_json::to_string(&no.agents[0]).unwrap(),
            r#"{"name":"a","roles":[]}"#
        );
        // whoever installs the plugin reviews it: it is part of the digest
        assert_ne!(bare.digest(), admin.digest());
        assert_eq!(
            serde_json::to_string(&admin.agents[0]).unwrap(),
            r#"{"name":"a","roles":[],"admin":true}"#
        );
        // a base role is admin or nothing: never owner
        assert!(Manifest::from_json(br#"{"agents":[{"name":"a","admin":"owner"}]}"#).is_err());
        assert!(Manifest::from_json(br#"{"agents":[{"name":"a","role":"owner"}]}"#).is_err());
    }

    #[test]
    fn agents_only_own_roles() {
        let err = Manifest::from_json(br#"{"agents":[{"name":"a","roles":["workspace.admin"]}]}"#)
            .unwrap_err();
        assert!(err.contains("not declared"));
    }
}
