// Declarations a plugin writes: messages, entities, descriptions, and the plugin's identity (roles, scopes,
// capabilities). The SDK reads them by reflection; `partial` is accepted, not required.
namespace Celobus.Sdk;

/// <summary>What a <see cref="MessageAttribute"/> type is used for (the descriptor's <c>kind</c>).</summary>
public enum MessageKind
{
    Message,
    Event,
    Job,
    Settings,
}

/// <summary>
/// A message of the plugin's contract: a record (or class) whose public properties are its fields, or an enum whose
/// members are its values (snake_case). The protobuf message name is the type name.
/// </summary>
[AttributeUsage(AttributeTargets.Class | AttributeTargets.Enum, Inherited = false)]
public sealed class MessageAttribute : Attribute
{
    public MessageKind Kind { get; set; } = MessageKind.Message;
}

/// <summary>
/// On a <see cref="MessageAttribute"/> class (a view a screen reads often): its fields at their protobuf default —
/// <c>""</c>, <c>0</c>, <c>false</c>, an empty list, a message whose JSON is <c>{}</c> — are left out of the JSON it
/// is written as. An enum and an optional (nullable) field that has a value are still written; readers take a missing
/// field as its default, so only the wire changes. Not for entities.
/// </summary>
[AttributeUsage(AttributeTargets.Class, Inherited = false)]
public sealed class OmitDefaultsAttribute : Attribute;

/// <summary>Who makes an entity's key <c>id</c>.</summary>
public enum KeyType
{
    /// <summary>A UUIDv7 made by the store (the default).</summary>
    Uuid,

    /// <summary>An identity made by the store.</summary>
    I64,

    /// <summary>A natural key the author supplies.</summary>
    String,
}

/// <summary>
/// An entity: a record whose fields are its columns, with an <c>Id</c> of type <c>Id&lt;Self&gt;</c>. Its name is the
/// snake_case type name (<c>OrderLine</c> → <c>order_line</c>); it is also a message. With <see cref="Plugin"/>, it is
/// another plugin's entity this plugin reads (<c>ctx.Query&lt;T&gt;()</c>), as the client <c>celobus plugin add</c>
/// generates declares it: never an entity of this plugin.
/// </summary>
[AttributeUsage(AttributeTargets.Class, Inherited = false)]
public sealed class EntityAttribute : Attribute
{
    /// <summary>The provider's plugin id, for a dependency's entity (null: an entity of this plugin).</summary>
    public string? Plugin { get; set; }

    public KeyType Key { get; set; } = KeyType.Uuid;
    public bool SoftDelete { get; set; }
    public bool Version { get; set; }
    public bool AppendOnly { get; set; }

    /// <summary>The property that names a row (<c>nameof(Number)</c>); default <c>Name</c>, <c>Title</c>, <c>Number</c>, else <c>Id</c>.</summary>
    public string? Label { get; set; }

    /// <summary>
    /// <see cref="EntityStorage.Olap"/>: the rows are kept in the analytic copy only, filled by the entity's load
    /// (<c>.Load&lt;T&gt;(..)</c>) when the plugin asks (<c>ctx.Analytics().Snapshot().Load&lt;T&gt;().RequestAsync()</c>)
    /// and read with <c>ctx.Analytics().Query&lt;T&gt;()</c> only (<c>ctx.Db&lt;T&gt;()</c> and <c>ctx.Query&lt;T&gt;()</c>
    /// are refused: CELOBUS061 at compile time). Never written; no history, <c>SoftDelete</c>, <c>Version</c> nor
    /// <c>AppendOnly</c>.
    /// </summary>
    public EntityStorage Storage { get; set; }

    /// <summary>
    /// With <see cref="EntityStorage.Olap"/>: the properties that key a row (<c>[nameof(Day), nameof(Store)]</c>: required
    /// plain values), unique among the rows a load gives; the row's <c>Id</c> is a uuid made from their values.
    /// </summary>
    public string[] OlapKey { get; set; } = [];
}

/// <summary>Where an entity's rows are kept.</summary>
public enum EntityStorage
{
    /// <summary>The workspace's database (and the analytic copy, as every entity).</summary>
    Default,

    /// <summary>The analytic copy only, filled by the entity's load from the Data Lake.</summary>
    Olap,
}

/// <summary>
/// The row of an analytic view of the plugin: a record whose public properties are its columns, declared with
/// <c>.View(from x in Views.From&lt;T&gt;() .. select new Sales(..))</c> and read with <c>ctx.Query&lt;Sales&gt;()</c> or
/// <c>ctx.Analytics().Query&lt;Sales&gt;()</c>, never written. Its name is the type's in snake_case unless
/// <see cref="Name"/> sets it; its <c>[Describe]</c> is the view's description (an agent of the plugin reads it to pick
/// the view).
/// </summary>
[AttributeUsage(AttributeTargets.Class, Inherited = false)]
public sealed class ViewAttribute : Attribute
{
    /// <summary>The view's name (snake_case); null: the type's name in snake_case.</summary>
    public string? Name { get; set; }
}

/// <summary>
/// An entity's history in the workspace's Data Lake (History.cs). On the entity, <c>[History("audit")]</c>: it keeps its
/// history in that layer of the lake (one the plugin declares with <c>.Layer(..)</c>): every change of its kept columns,
/// each its own row with when it was made and by whom. On a column, <c>[History]</c>: the history keeps that column's
/// changes. With <see cref="All"/> every column is kept, but the encrypted ones, the lists and messages and, unless
/// <see cref="Unmasked"/> is said too, the masked ones. Who reads it: <c>.HistoryRead&lt;T&gt;(roles)</c>,
/// <c>.HistoryReadAsSystem&lt;T&gt;()</c>, <c>.HistoryExport&lt;T&gt;()</c>.
/// </summary>
[AttributeUsage(AttributeTargets.Class | AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class HistoryAttribute : Attribute
{
    /// <summary>On a column: the entity's history keeps its changes.</summary>
    public HistoryAttribute() { }

    /// <summary>On the entity: the layer of the lake its history goes to.</summary>
    public HistoryAttribute(string layer) => Layer = layer;

    /// <summary>The layer (on the entity).</summary>
    public string? Layer { get; }

    /// <summary>
    /// Every column is kept, the ones a later version adds too, but the encrypted ones, the lists and messages and,
    /// unless <see cref="Unmasked"/>, the masked ones (<c>celobus plugin check</c> lists what it left out).
    /// </summary>
    public bool All { get; set; }

    /// <summary>Masked columns may be kept: their values go to the lake as they are (the lake has no masks).</summary>
    public bool Unmasked { get; set; }
}

/// <summary>An entity column whose values are unique (among the rows that are not deleted, on a soft-delete entity).</summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class UniqueAttribute : Attribute;

/// <summary>
/// An entity column the platform keeps encrypted at rest with the workspace's key (never unique, indexed, filtered,
/// sorted, a ref, the id, the label or read by declared SQL).
/// </summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class EncryptedAttribute : Attribute;

/// <summary>
/// An index of an entity's table. On a property: an index of that column. On the entity: an index over several
/// columns in order, each a property name or a system column, optionally followed by <c>desc</c> or <c>asc</c>
/// (<c>[Index(nameof(OwnerId), "CreatedAt desc")]</c>); <c>Unique = true</c> makes it a unique one.
/// </summary>
[AttributeUsage(AttributeTargets.Class | AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false, AllowMultiple = true)]
public sealed class IndexAttribute(params string[] columns) : Attribute
{
    public IReadOnlyList<string> Columns { get; } = columns;
    public bool Unique { get; set; }
}

/// <summary>
/// A <c>string</c> entity column searched by full text (<c>Filter&lt;T&gt;.Search</c>), in a PostgreSQL text search
/// language (<c>[Searchable("english")]</c>; <c>simple</c> by default). At most 8 per entity; the platform indexes
/// it for full-text and substring search.
/// </summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class SearchableAttribute(string language = "simple") : Attribute
{
    public string Language { get; } = language;
}

/// <summary>
/// The navigations of a ref column (<c>Id&lt;Target&gt;</c>): <c>Nav</c>, the forward one (default: the column's
/// snake_case name without <c>_id</c>), and <c>Inverse</c>, the one on the target (an entity of this plugin) that
/// lists the rows pointing at it; both are what <c>.Include(..)</c> names. <c>OnDelete</c>: what the ref does when its
/// target is deleted for good (<see cref="Sdk.OnDelete"/>).
/// <c>[Ref(Nav = "order", Inverse = "lines", OnDelete = OnDelete.Cascade)] Id&lt;Order&gt; OrderId</c>.
/// </summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class RefAttribute : Attribute
{
    public string? Nav { get; set; }
    public string? Inverse { get; set; }
    public OnDelete OnDelete { get; set; }
}

/// <summary>
/// What a ref does when the row it names is deleted for good. Within the plugin: <c>Restrict</c> (the default: the
/// target is not deleted while rows refer to it), <c>Cascade</c> (they are deleted with it), <c>SetNull</c> (an
/// optional ref is cleared), <c>NoAction</c> (as <c>Restrict</c>, checked at the end of the statement); to another
/// plugin's entity or the directory: <c>Keep</c> (the default: the id stays) or, onto the directory, <c>SetNull</c>.
/// A soft delete of the target does nothing: the rows go (or are cleared) when it is removed for good.
/// </summary>
public enum OnDelete
{
    /// <summary>Not declared: the default of the ref's kind.</summary>
    Default,
    Restrict,
    Cascade,
    SetNull,
    NoAction,
    Keep,
}

/// <summary>The description of a message, entity, field or enum in the descriptor.</summary>
[AttributeUsage(AttributeTargets.Class | AttributeTargets.Enum | AttributeTargets.Property | AttributeTargets.Parameter |
                AttributeTargets.Field, Inherited = false)]
public sealed class DescribeAttribute(string text) : Attribute
{
    public string Text { get; } = text;
}

/// <summary>
/// A ref target outside the plugin: the directory's <c>platform.user</c> (the SDK's own marker types), or another
/// plugin's entity (<c>[RefTarget("catalog.product")]</c> on the marker classes of a generated dependency client), so
/// <c>Id&lt;T&gt;</c> of the marker is a <c>ref:catalog.product</c>.
/// </summary>
[AttributeUsage(AttributeTargets.Class, Inherited = false)]
public sealed class RefTargetAttribute(string target) : Attribute
{
    public string Target { get; } = target;
}

/// <summary>
/// A dependency's entity this plugin reads (<c>ctx.Query&lt;T&gt;()</c>) and the fields it reads, as the client
/// <c>celobus plugin add</c> generates lists them; declared with <c>.Reads(plugin, reads)</c>.
/// </summary>
public sealed record DependencyRead(string Entity, ReadField[] Fields);

/// <summary>A field of a dependency's entity this plugin reads, with its descriptor type (<c>optional string</c>).</summary>
public sealed record ReadField(string Name, string Type);

/// <summary>Which members a plugin role is given by default.</summary>
public enum RoleDefault
{
    None,
    Member,
    Admin,
}

/// <summary>
/// A plugin role: <c>new("manager") { Title = "Order manager", Strict = true }</c>. Declared on the plugin with
/// <c>.Role(..)</c>; an endpoint names the roles it admits. <c>Strict</c>: only an explicit assignment counts,
/// <c>workspace.admin</c> does not substitute.
/// </summary>
public sealed record Role(string Name)
{
    public string Title { get; init; } = "";
    public string Description { get; init; } = "";
    public bool Strict { get; init; }
    public RoleDefault DefaultFor { get; init; } = RoleDefault.None;

    /// <summary>The platform's base roles (a higher one implies the lower ones).</summary>
    public static readonly Role WorkspaceOwner = new("workspace.owner");

    public static readonly Role WorkspaceAdmin = new("workspace.admin");
    public static readonly Role WorkspaceMember = new("workspace.member");
    public static readonly Role WorkspaceGuest = new("workspace.guest");

    internal bool IsPlatform => Name.StartsWith("workspace.", StringComparison.Ordinal);
}

/// <summary>
/// A scope of the plugin's API: <c>Scope.Read</c>, <c>Scope.Write</c> (implicit; declare them to give them a title)
/// or <c>new Scope("export")</c>. The title is the consent text.
/// </summary>
public sealed record Scope(string Name)
{
    public string Title { get; init; } = "";
    public string Description { get; init; } = "";
    public bool Sensitive { get; init; }

    public static readonly Scope Read = new("read");
    public static readonly Scope Write = new("write");
}

/// <summary>A power beyond the plugin's own data, approved at install; declared with a reason.</summary>
public sealed record Capability(string Name)
{
    public static readonly Capability AuthzBypass = new("authz:bypass");
    public static readonly Capability IdentityManage = new("identity:manage");
    public static readonly Capability PushSend = new("push:send");
    public static readonly Capability SqlWrite = new("sql:write");
    public static readonly Capability SqlUnsafe = new("sql:unsafe");
    public static readonly Capability BlobsUse = new("blobs:use");
    public static readonly Capability McpTools = new("mcp:tools");

    internal static readonly HashSet<string> Known =
    [
        "authz:bypass", "identity:manage", "push:send", "sql:write", "sql:unsafe", "blobs:use", "mcp:tools",
    ];
}

/// <summary>The coding-agent CLI that runs an agent.</summary>
public enum AgentEngine
{
    /// <summary>Claude Code: the engine of an agent that names none.</summary>
    Claude,
    Codex,
    Pi,
}

/// <summary>What a connection connects to.</summary>
public enum ConnectionKind
{
    /// <summary>A language model provider, for the plugin's agents (and its dependents').</summary>
    Llm,
    /// <summary>Speech recognition, for the workspace's dictation.</summary>
    Stt,
}

/// <summary>
/// A connection the plugin declares: installing the plugin creates it, not set up, in the
/// workspace, and an owner or admin fills it in (provider, key, models). An agent names an <c>llm</c> one
/// (<c>AgentOptions.Connection</c>); the plugin's code never sees it:
/// <c>.Connection(Connection.Llm("anthropic").Title("Shared Anthropic"))</c>.
/// </summary>
public sealed class Connection
{
    private Connection(string name, ConnectionKind kind) { Name = name; Kind = kind; }

    internal string Name { get; }
    internal ConnectionKind Kind { get; }
    internal string TitleText { get; private set; } = "";
    internal string Description { get; private set; } = "";

    /// <summary>A language model provider for agents, named <paramref name="name"/> (snake_case).</summary>
    public static Connection Llm(string name) => new(name, ConnectionKind.Llm);

    /// <summary>Speech recognition for dictation, named <paramref name="name"/> (snake_case).</summary>
    public static Connection Stt(string name) => new(name, ConnectionKind.Stt);

    /// <summary>Its display name (at most 80 characters).</summary>
    public Connection Title(string title)
    {
        TitleText = title;
        return this;
    }

    /// <summary>What it is for, in a sentence (at most 500 characters).</summary>
    public Connection Describe(string text)
    {
        Description = text;
        return this;
    }
}

/// <summary>The options of an agent, set in <c>Plugin.Agent(name, a =&gt; …)</c>.</summary>
public sealed class AgentOptions
{
    internal AgentOptions(string name) { Name = name; }

    internal string Name { get; }
    internal List<string> RoleList { get; } = [];
    internal List<string> WorkspaceList { get; } = [];
    internal List<string> DelegateList { get; } = [];
    internal string ConnectionName { get; private set; } = "";
    internal bool HasSshKey { get; private set; }
    internal bool IsAdmin { get; private set; }
    internal string TitleText { get; private set; } = "";
    internal string Description { get; private set; } = "";
    internal string IconName { get; private set; } = "";
    internal AgentEngine? EngineValue { get; private set; }
    internal string ModelName { get; private set; } = "";
    internal bool IsSession { get; private set; }
    internal TimeSpan? TimeoutValue { get; private set; }
    internal List<string> SkillList { get; } = [];
    internal string TopicName { get; private set; } = "";

    /// <summary>The plugin's roles the agent holds.</summary>
    public AgentOptions Roles(params Role[] roles)
    {
        RoleList.AddRange(roles.Select(r => r.Name));
        return this;
    }

    /// <summary>
    /// The workspace permissions the agent asks for (<c>"plugins.install"</c>): with one, the agent does the
    /// workspace operations that take it as a workspace admin would. Whoever installs the plugin sees the list.
    /// </summary>
    public AgentOptions Workspace(params string[] permissions)
    {
        WorkspaceList.AddRange(permissions);
        return this;
    }

    /// <summary>
    /// The agents this one may bring into its group when they are not there: an agent of this plugin by its name, or
    /// <c>&lt;dependency&gt;.&lt;agent&gt;</c>. Messaging an agent that is in the group already needs no entry.
    /// Whoever installs the plugin sees the list.
    /// </summary>
    public AgentOptions Delegates(params string[] agents)
    {
        DelegateList.AddRange(agents);
        return this;
    }

    /// <summary>
    /// The <c>llm</c> connection its runs use: a connection of this plugin by its name (<c>"research"</c>), or
    /// <c>&lt;dependency&gt;.&lt;name&gt;</c> (<c>"ai.anthropic"</c>). An owner or admin may choose another one of the
    /// plugin or its dependencies in the agent's settings. Whoever installs the plugin sees it.
    /// </summary>
    public AgentOptions Connection(string connection)
    {
        ConnectionName = connection;
        return this;
    }

    /// <summary>
    /// The agent has an SSH key of its own (ed25519, one per workspace), in <c>~/.ssh</c> of its runs: its public key is
    /// shown in the workspace's settings, to add to the agent's account at GitHub or GitLab.
    /// </summary>
    public AgentOptions SshKey()
    {
        HasSshKey = true;
        return this;
    }

    /// <summary>
    /// The agent's service account asks for base role admin instead of member: it does whatever a workspace admin
    /// does, not only the operations of its <see cref="Workspace"/> permissions. Only an owner installs or upgrades to
    /// a version that asks for it, and whoever installs the plugin sees it; an owner may still change the account's
    /// role. Never owner.
    /// </summary>
    public AgentOptions Admin()
    {
        IsAdmin = true;
        return this;
    }

    /// <summary>The agent's display name.</summary>
    public AgentOptions Title(string title)
    {
        TitleText = title;
        return this;
    }

    /// <summary>What the agent is for, in a sentence.</summary>
    public AgentOptions Describe(string text)
    {
        Description = text;
        return this;
    }

    /// <summary>
    /// The agent's own glyph, a lucide icon's name (<c>"rocket"</c>, <c>"git-pull-request"</c>): the client draws it over
    /// its engine's mark wherever the agent is shown, so two agents on one engine are told apart.
    /// </summary>
    public AgentOptions Icon(string icon)
    {
        IconName = icon;
        return this;
    }

    /// <summary>
    /// The engine of its runs, unless whoever starts one names another (default: the one its connection's protocol
    /// picks: Claude Code for <c>anthropic</c>, Codex for <c>openai_responses</c>, Pi for <c>openai_chat</c>).
    /// </summary>
    public AgentOptions Engine(AgentEngine engine)
    {
        EngineValue = engine;
        return this;
    }

    /// <summary>
    /// The model of its runs when its connection lists it, unless whoever starts a run or the workspace's settings name
    /// another (default: the connection's default model).
    /// </summary>
    public AgentOptions Model(string model)
    {
        ModelName = model;
        return this;
    }

    /// <summary>Its runs are sessions: they wait for follow-up messages after every turn.</summary>
    public AgentOptions Session()
    {
        IsSession = true;
        return this;
    }

    /// <summary>The longest a run takes, in whole seconds from a minute to six hours (default: 30 minutes).</summary>
    public AgentOptions Timeout(TimeSpan timeout)
    {
        TimeoutValue = timeout;
        return this;
    }

    /// <summary>The shared skills of the plugin (<c>skills/&lt;name&gt;/</c>) the agent has besides its own.</summary>
    public AgentOptions Skills(params string[] skills)
    {
        SkillList.AddRange(skills);
        return this;
    }

    /// <summary>
    /// A topic of the plugin (declared with <c>Plugin.Topic</c>) the platform invalidates, for every key, whenever a run
    /// of the agent changes: its status, a new event, the text being written. Screens that follow it re-read the run
    /// through the plugin's endpoints (<c>ctx.Agents.Get</c>, <c>ctx.Agents.Events</c>).
    /// </summary>
    public AgentOptions Topic<TKey>(Ui.Topic<TKey> topic) where TKey : class
    {
        TopicName = topic.Name;
        return this;
    }
}
