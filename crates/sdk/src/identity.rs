//! Identity declarations: roles, scopes, capabilities and agents.

/// A role of the plugin (`<plugin>.<name>` outside code), or a base role of the workspace
/// (`Role::WORKSPACE_ADMIN`, `workspace.admin`).
///
/// ```
/// use celobus_sdk::prelude::*;
/// pub const MANAGER: Role = Role::new("manager").title("Order manager").strict();
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Role {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    strict: bool,
    default_for: DefaultFor,
}

/// Who gets a role without being assigned it: nobody (default), every member, or admins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DefaultFor {
    #[default]
    None,
    Member,
    Admin,
}

impl DefaultFor {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            DefaultFor::None => "none",
            DefaultFor::Member => "member",
            DefaultFor::Admin => "admin",
        }
    }
}

impl Role {
    pub const WORKSPACE_OWNER: Role = Role::new("workspace.owner");
    pub const WORKSPACE_ADMIN: Role = Role::new("workspace.admin");
    pub const WORKSPACE_MEMBER: Role = Role::new("workspace.member");
    pub const WORKSPACE_GUEST: Role = Role::new("workspace.guest");

    /// A role of this plugin: `^[a-z][a-z0-9_]{1,31}$`.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            title: "",
            description: "",
            strict: false,
            default_for: DefaultFor::None,
        }
    }

    pub const fn title(mut self, title: &'static str) -> Self {
        self.title = title;
        self
    }

    pub const fn describe(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    /// Only an explicit assignment grants it: `workspace.admin` does not substitute.
    pub const fn strict(mut self) -> Self {
        self.strict = true;
        self
    }

    pub const fn default_for(mut self, who: DefaultFor) -> Self {
        self.default_for = who;
        self
    }

    /// The name as declared: `manager`, or `workspace.admin` for a base role.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn get_title(&self) -> &'static str {
        self.title
    }

    pub const fn description(&self) -> &'static str {
        self.description
    }

    pub const fn is_strict(&self) -> bool {
        self.strict
    }

    pub const fn get_default_for(&self) -> DefaultFor {
        self.default_for
    }

    /// A base role of the workspace (`workspace.*`), not one the plugin declares.
    pub fn is_workspace(&self) -> bool {
        self.name.starts_with("workspace.")
    }

    /// The role as the caller holds it: `<plugin>.<name>`, or the name when it is already full.
    pub(crate) fn full_name(&self, plugin: &str) -> String {
        if self.name.contains('.') {
            self.name.to_string()
        } else {
            format!("{plugin}.{}", self.name)
        }
    }
}

/// A scope of the plugin (`<plugin>:<name>` in tokens). `read` and `write` always exist; declare them
/// only to give them a title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    sensitive: bool,
}

impl Scope {
    pub const READ: Scope = Scope::new("read");
    pub const WRITE: Scope = Scope::new("write");

    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            title: "",
            description: "",
            sensitive: false,
        }
    }

    /// Consent text.
    pub const fn title(mut self, title: &'static str) -> Self {
        self.title = title;
        self
    }

    pub const fn describe(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    /// Asked for separately at consent.
    pub const fn sensitive(mut self) -> Self {
        self.sensitive = true;
        self
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn get_title(&self) -> &'static str {
        self.title
    }

    pub const fn description(&self) -> &'static str {
        self.description
    }

    pub const fn is_sensitive(&self) -> bool {
        self.sensitive
    }

    pub(crate) fn full_name(&self, plugin: &str) -> String {
        if self.name.contains(':') {
            self.name.to_string()
        } else {
            format!("{plugin}:{}", self.name)
        }
    }
}

/// A power beyond the plugin's own data, approved at install; declared with a reason
/// (`.capability(Capability::PUSH_SEND, "Reminds clerks of their unpaid orders.")`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    name: &'static str,
}

impl Capability {
    pub const AUTHZ_BYPASS: Capability = Capability::new("authz:bypass");
    pub const IDENTITY_MANAGE: Capability = Capability::new("identity:manage");
    pub const PUSH_SEND: Capability = Capability::new("push:send");
    pub const SQL_WRITE: Capability = Capability::new("sql:write");
    pub const SQL_UNSAFE: Capability = Capability::new("sql:unsafe");
    pub const BLOBS_USE: Capability = Capability::new("blobs:use");
    pub const MCP_TOOLS: Capability = Capability::new("mcp:tools");

    /// The capabilities the platform knows (`E_CAPABILITY_UNKNOWN` for any other).
    pub const KNOWN: &'static [Capability] = &[
        Self::AUTHZ_BYPASS,
        Self::IDENTITY_MANAGE,
        Self::PUSH_SEND,
        Self::SQL_WRITE,
        Self::SQL_UNSAFE,
        Self::BLOBS_USE,
        Self::MCP_TOOLS,
    ];

    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }
}

/// The coding-agent CLI that runs an agent (`.engine(AgentEngine::Codex)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentEngine {
    /// Claude Code: the engine of an agent that names none.
    Claude,
    Codex,
    Pi,
}

impl AgentEngine {
    pub const fn name(self) -> &'static str {
        match self {
            AgentEngine::Claude => "claude",
            AgentEngine::Codex => "codex",
            AgentEngine::Pi => "pi",
        }
    }
}

/// What a connection connects to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    /// A language model provider, for the plugin's agents (and its dependents').
    Llm,
    /// Speech recognition, for the workspace's dictation.
    Stt,
    /// A basemap provider, for the plugin's maps (and its dependents').
    Maps,
}

impl ConnectionKind {
    pub const fn name(self) -> &'static str {
        match self {
            ConnectionKind::Llm => "llm",
            ConnectionKind::Stt => "stt",
            ConnectionKind::Maps => "maps",
        }
    }
}

/// A connection the plugin declares: installing the plugin creates it, not set up, in the
/// workspace, and an owner or admin fills it in (provider, key, models). An agent names an `llm` one
/// (`Agent::connection`); the plugin's code never sees it.
///
/// ```
/// use celobus_sdk::prelude::*;
/// let anthropic = Connection::llm("anthropic")
///     .title("Shared Anthropic")
///     .describe("The Anthropic account the workspace's agents run on.");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub(crate) name: String,
    pub(crate) kind: ConnectionKind,
    pub(crate) title: String,
    pub(crate) description: String,
}

impl Connection {
    /// A connection named `name` (snake_case) of this kind.
    pub fn new(name: impl Into<String>, kind: ConnectionKind) -> Self {
        Self {
            name: name.into(),
            kind,
            title: String::new(),
            description: String::new(),
        }
    }

    /// A language model provider for agents.
    pub fn llm(name: impl Into<String>) -> Self {
        Self::new(name, ConnectionKind::Llm)
    }

    /// Speech recognition for dictation.
    pub fn stt(name: impl Into<String>) -> Self {
        Self::new(name, ConnectionKind::Stt)
    }

    /// A basemap provider for maps: a map (`viz::geo::map().connection(..)`) names it. Not set up, it draws
    /// OpenStreetMap; an owner or admin may choose another provider and give its key.
    pub fn maps(name: impl Into<String>) -> Self {
        Self::new(name, ConnectionKind::Maps)
    }

    /// Its display name (at most 80 characters).
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// What it is for, in a sentence (at most 500 characters).
    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
}

/// An agent of the plugin: a service account with the roles it holds, and how its runs are made. Its system
/// prompt, its skills and its extra MCP servers are files of the plugin: `agents/<name>/soul.md`,
/// `agents/<name>/skills/<skill>/SKILL.md`, `agents/<name>/mcp.json`.
///
/// ```
/// use celobus_sdk::prelude::*;
/// pub const MANAGER: Role = Role::new("manager");
/// let assistant = Agent::new("assistant")
///     .roles([MANAGER])
///     .describe("Answers questions about products and prices.")
///     .engine(AgentEngine::Claude)
///     .skills(["money"]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub(crate) name: String,
    pub(crate) roles: Vec<String>,
    pub(crate) workspace: Vec<String>,
    pub(crate) delegates: Vec<String>,
    pub(crate) connection: String,
    pub(crate) ssh_key: bool,
    pub(crate) admin: bool,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) icon: String,
    pub(crate) engine: Option<AgentEngine>,
    pub(crate) model: String,
    pub(crate) session: bool,
    pub(crate) timeout: Option<std::time::Duration>,
    pub(crate) skills: Vec<String>,
    pub(crate) topic: String,
}

impl Agent {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            roles: Vec::new(),
            workspace: Vec::new(),
            delegates: Vec::new(),
            connection: String::new(),
            ssh_key: false,
            admin: false,
            title: String::new(),
            description: String::new(),
            icon: String::new(),
            engine: None,
            model: String::new(),
            session: false,
            timeout: None,
            skills: Vec::new(),
            topic: String::new(),
        }
    }

    /// The workspace permissions the agent asks for (`"plugins.install"`, `"members.write"`, …): with one, the
    /// agent does the workspace operations that take it as a workspace admin would. Whoever installs the plugin
    /// sees the list; without any, the agent is an ordinary member.
    pub fn workspace<S: Into<String>>(mut self, permissions: impl IntoIterator<Item = S>) -> Self {
        self.workspace
            .extend(permissions.into_iter().map(Into::into));
        self
    }

    /// The agents this one may bring into its group when they are not there: an agent of this plugin by its
    /// name (`"release"`), or `<dependency>.<agent>` (`"crm.scout"`). Messaging an agent that is in the group
    /// already needs no entry. Whoever installs the plugin sees the list.
    pub fn delegates<S: Into<String>>(mut self, agents: impl IntoIterator<Item = S>) -> Self {
        self.delegates.extend(agents.into_iter().map(Into::into));
        self
    }

    /// The `llm` connection its runs use: a connection of this plugin by its name (`"research"`), or
    /// `<dependency>.<name>` (`"ai.anthropic"`). An owner or admin may choose another one of the plugin or its
    /// dependencies in the agent's settings. Whoever installs the plugin sees it.
    pub fn connection(mut self, connection: impl Into<String>) -> Self {
        self.connection = connection.into();
        self
    }

    /// The agent has an SSH key of its own (ed25519, one per workspace), in `~/.ssh` of its runs: its public key is
    /// shown in the workspace's settings, to add to the agent's account at GitHub or GitLab.
    pub fn ssh_key(mut self) -> Self {
        self.ssh_key = true;
        self
    }

    /// The agent's service account asks for base role admin instead of member: it does whatever a workspace admin
    /// does, not only the operations of its [`workspace`](Self::workspace) permissions. Only an owner installs or
    /// upgrades to a version that asks for it, and whoever installs the plugin sees it; an owner may still change the
    /// account's role. Never owner.
    pub fn admin(mut self) -> Self {
        self.admin = true;
        self
    }

    pub fn roles(mut self, roles: impl IntoIterator<Item = Role>) -> Self {
        self.roles
            .extend(roles.into_iter().map(|r| r.name().to_string()));
        self
    }

    /// The agent's display name.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// What the agent is for, in a sentence.
    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// The agent's own glyph, a lucide icon's name (`"rocket"`, `"git-pull-request"`): the client draws it over its
    /// engine's mark wherever the agent is shown, so two agents on one engine are told apart.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = icon.into();
        self
    }

    /// The engine of the agent's runs, unless whoever starts one names another (default: the one its connection's
    /// protocol picks: Claude Code for `anthropic`, Codex for `openai_responses`, Pi for `openai_chat`).
    pub fn engine(mut self, engine: AgentEngine) -> Self {
        self.engine = Some(engine);
        self
    }

    /// The model of its runs (`"claude-opus-5-5"`) when its connection lists it, unless whoever starts a run or the
    /// workspace's settings name another (default: the connection's default model).
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Runs of this agent are sessions: they wait for follow-up messages after every turn.
    pub fn session(mut self) -> Self {
        self.session = true;
        self
    }

    /// The longest a run takes, in whole seconds from a minute to six hours (default: 30 minutes).
    pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// The shared skills of the plugin (`skills/<name>/`) the agent has besides its own.
    pub fn skills<S: Into<String>>(mut self, skills: impl IntoIterator<Item = S>) -> Self {
        self.skills.extend(skills.into_iter().map(Into::into));
        self
    }

    /// A topic of the plugin (declared with `Plugin::topic`) the platform invalidates, for every key, whenever a
    /// run of the agent changes: its status, a new event, the text being written. Screens that follow the topic
    /// re-read the run through the plugin's endpoints (`ctx.agents().get(..)`, `.events(..)`).
    pub fn topic<K: crate::message::Message>(mut self, topic: crate::ui::Topic<K>) -> Self {
        self.topic = topic.name().to_string();
        self
    }
}
