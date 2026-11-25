// The plugin: its identity and subjects, `--describe`, and serving.
//
//   public static Plugin Define() => Plugin.Create("hello")      // the version is <Version> of the project
//       .Title("Hello")
//       .Role(Roles.Clerk)
//       .Endpoint<Hello>();
//   // Program.cs
//   await HelloPlugin.Define().Serve();
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace Celobus.Sdk;

/// <summary>What a plugin declares (the input of the descriptor and of the runtime).</summary>
internal sealed class PluginDefinition(string id, string version)
{
    public string Id { get; } = id;
    public string Version { get; set; } = version;
    public string? Title { get; set; }
    public string Description { get; set; } = "";
    public List<Role> Roles { get; } = [];
    public List<Scope> Scopes { get; } = [];
    public List<(Capability Capability, string Reason)> Capabilities { get; } = [];
    public List<string> Permissions { get; } = [];
    public List<AgentOptions> Agents { get; } = [];
    public List<Connection> Connections { get; } = [];
    public List<DependencyDecl> Dependencies { get; } = [];
    public List<NotificationKind> Notifications { get; } = [];
    public List<Type> Entities { get; } = [];
    public List<(Type Entity, System.Text.Json.Nodes.JsonObject Rules)> Accesses { get; } = [];
    public List<(Type Entity, string[] Ops)> Bypass { get; } = [];
    public List<Type> Messages { get; } = [];
    public List<EndpointRegistration> Endpoints { get; } = [];
    public List<Ui.TopicDecl> Topics { get; } = [];
    public Ui.PluginUi? Ui { get; set; }
    public List<JobRegistration> Jobs { get; } = [];

    /// <summary>The analytic views: each its row type and its query (<c>.View(..)</c>).</summary>
    public List<(Type Row, System.Linq.Expressions.Expression Query)> Views { get; } = [];

    /// <summary>The Data Lake: the layers, the tables' rows, who reads them, other plugins' tables read (Lake.cs).</summary>
    public List<Layer> Layers { get; } = [];
    public List<Type> LakeTables { get; } = [];
    public List<(Type Table, Role[] Roles)> LakeReads { get; } = [];
    public List<Type> LakeSystemReads { get; } = [];
    public List<Type> LakeExports { get; } = [];
    public List<Type> LakeModelExports { get; } = [];

    /// <summary>
    /// Other plugins' lake tables (<c>.LakeDependency&lt;T&gt;()</c>) and entities whose histories it reads
    /// (<c>.HistoryDependency&lt;T&gt;()</c>), in the order declared: each dependency's <c>lake</c> lists them so.
    /// </summary>
    public List<Type> LakeDependencies { get; } = [];

    /// <summary>The models of lake tables (<c>.Model(..)</c>): each its table, its query, whether it copies masked columns.</summary>
    public List<(Type Table, System.Linq.Expressions.Expression Query, bool Unmasked)> LakeModels { get; } = [];

    /// <summary>The loads of the entities stored in OLAP (<c>.Load&lt;T&gt;(..)</c>): each its entity and its query.</summary>
    public List<(Type Entity, System.Linq.Expressions.Expression Query)> Loads { get; } = [];

    /// <summary>Who reads the entities' histories (History.cs).</summary>
    public List<(Type Entity, Role[] Roles)> HistoryReads { get; } = [];
    public List<Type> HistorySystemReads { get; } = [];
    public List<Type> HistoryExports { get; } = [];

    /// <summary>How many runs of its jobs a process runs at once (<c>.JobSlots</c>).</summary>
    public int JobSlots { get; set; } = DefaultJobSlots;

    /// <summary>The runs a process starts at once by default, and the most it may announce.</summary>
    public const int DefaultJobSlots = 16;
    public const int MaxJobSlots = 64;

    /// <summary>The run-change hook (<c>.OnRunChange</c>).</summary>
    public Func<Ctx, RunChange, Task>? OnRunChange { get; set; }
}

/// <summary>
/// A plugin this one depends on: the endpoints it calls (<c>.DependsOn</c>) and the entities it reads
/// (<c>.Reads</c>). <c>CallsDeclared</c>: <c>.DependsOn</c> made or filled it (an entry made by <c>.Reads</c> alone is
/// filled by a later <c>.DependsOn</c>, not repeated).
/// </summary>
internal sealed class DependencyDecl(string plugin)
{
    public string Plugin { get; } = plugin;
    public List<string> Calls { get; } = [];
    public List<DependencyRead> Reads { get; } = [];
    public bool CallsDeclared { get; set; }
}

/// <summary>A plugin: <c>Plugin.Create("orders").Title("Orders").Role(..).Endpoint&lt;PlaceOrder&gt;()</c>, then <c>await plugin.Serve()</c>.</summary>
public sealed class Plugin
{
    private readonly PluginDefinition _def;

    private Plugin(PluginDefinition def) { _def = def; }

    /// <summary>
    /// A plugin with this id; its version is the calling assembly's informational version (<c>&lt;Version&gt;</c> of the
    /// project, without build metadata), unless <see cref="Version"/> sets it.
    /// </summary>
    [MethodImpl(MethodImplOptions.NoInlining)]
    public static Plugin Create(string id) =>
        new(new PluginDefinition(id, AssemblyVersion(Assembly.GetCallingAssembly())));

    /// <summary>The plugin id.</summary>
    public string Id => _def.Id;

    internal PluginDefinition Definition => _def;

    public Plugin Version(string version)
    {
        _def.Version = version;
        return this;
    }

    /// <summary>The display title (default: the id, capitalized).</summary>
    public Plugin Title(string title)
    {
        _def.Title = title;
        return this;
    }

    public Plugin Describe(string description)
    {
        _def.Description = description;
        return this;
    }

    public Plugin Role(Role role)
    {
        _def.Roles.Add(role);
        return this;
    }

    /// <summary>Declares a scope (<c>Scope.Write with { Title = "Place orders" }</c>); <c>read</c> and <c>write</c> exist without it.</summary>
    public Plugin Scope(Scope scope)
    {
        _def.Scopes.Add(scope);
        return this;
    }

    /// <summary>Asks for a capability, with the reason shown when the plugin is installed.</summary>
    public Plugin Capability(Capability capability, string reason)
    {
        _def.Capabilities.Add((capability, reason));
        return this;
    }

    /// <summary>Directory data beyond names (<c>members.email</c>).</summary>
    public Plugin Permission(string permission)
    {
        _def.Permissions.Add(permission);
        return this;
    }

    /// <summary>A service agent of the plugin with these roles.</summary>
    public Plugin Agent(string name, params Role[] roles) => Agent(name, a => a.Roles(roles));

    /// <summary>
    /// A service agent with these roles and the workspace permissions it asks for
    /// (<c>.Agent("builder", ["plugins.install"], Builder)</c>): with one, the agent does the workspace operations
    /// that take it as a workspace admin would. Whoever installs the plugin sees the list.
    /// </summary>
    public Plugin Agent(string name, IEnumerable<string> workspace, params Role[] roles) =>
        Agent(name, a => a.Roles(roles).Workspace([.. workspace]));

    /// <summary>
    /// An agent and how its runs are made:
    /// <c>.Agent("assistant", a =&gt; a.Roles(Roles.Manager).Engine(AgentEngine.Claude).Describe("Answers price questions."))</c>.
    /// Its system prompt, its skills and its extra MCP servers are files of the plugin: <c>agents/&lt;name&gt;/soul.md</c>,
    /// <c>agents/&lt;name&gt;/skills/&lt;skill&gt;/SKILL.md</c>, <c>agents/&lt;name&gt;/mcp.json</c>.
    /// </summary>
    public Plugin Agent(string name, Action<AgentOptions> configure)
    {
        var agent = new AgentOptions(name);
        configure(agent);
        _def.Agents.Add(agent);
        return this;
    }

    /// <summary>
    /// A connection of the plugin: <c>.Connection(Connection.Llm("research").Title("Research model"))</c>.
    /// Installing the plugin creates it, not set up, in the workspace.
    /// </summary>
    public Plugin Connection(Connection connection)
    {
        _def.Connections.Add(connection);
        return this;
    }

    /// <summary>A plugin this one calls, with the endpoints it calls: <c>.DependsOn("customers", "GetCustomer")</c>.</summary>
    public Plugin DependsOn(string plugin, params string[] calls)
    {
        var dependency = _def.Dependencies.FirstOrDefault(d => d.Plugin == plugin && !d.CallsDeclared);
        if (dependency is null)
        {
            dependency = new DependencyDecl(plugin);
            _def.Dependencies.Add(dependency);
        }
        dependency.CallsDeclared = true;
        dependency.Calls.AddRange(calls);
        return this;
    }

    /// <summary>
    /// The entities of a dependency this plugin reads with <c>ctx.Query&lt;T&gt;()</c>, and their fields, as the
    /// generated client lists them: <c>.Reads(Deps.Customers.Plugin, Deps.Customers.Reads)</c>. Adds them to the
    /// dependency's entry (made, with no calls, when <c>.DependsOn</c> has not named the plugin).
    /// </summary>
    public Plugin Reads(string plugin, IEnumerable<DependencyRead> reads)
    {
        var dependency = _def.Dependencies.FirstOrDefault(d => d.Plugin == plugin);
        if (dependency is null)
        {
            dependency = new DependencyDecl(plugin);
            _def.Dependencies.Add(dependency);
        }
        dependency.Reads.AddRange(reads);
        return this;
    }

    /// <summary>
    /// Declares a kind of notification the plugin sends (<c>ctx.Notifications</c>); needs the capability
    /// <c>push:send</c> (<c>.Capability(Capability.PushSend, "&lt;why&gt;")</c>).
    /// </summary>
    public Plugin Notification(NotificationKind kind)
    {
        _def.Notifications.Add(kind);
        return this;
    }

    /// <summary>Declares an <c>[Entity]</c> record.</summary>
    public Plugin Entity<T>() where T : class
    {
        _def.Entities.Add(typeof(T));
        return this;
    }

    /// <summary>
    /// The access rules of an entity (<see cref="Access{T}"/>): without them no one but a system context reads or
    /// writes its rows.
    /// </summary>
    public Plugin Access<T>(Action<Access<T>> declare) where T : class
    {
        var access = new Access<T>();
        declare(access);
        _def.Accesses.Add((typeof(T), access.ToJson()));
        return this;
    }

    /// <summary>
    /// The scope of the <c>authz:bypass</c> capability: what a system context (<c>ctx.AsSystem(reason)</c>) may do on
    /// an entity, with <c>Perm.Read</c>, <c>Create</c>, <c>Update</c>, <c>Delete</c>.
    /// </summary>
    public Plugin Bypass<T>(params Perm[] ops) where T : class
    {
        _def.Bypass.Add((typeof(T), [.. ops.Select(o => o.Name)]));
        return this;
    }

    /// <summary>
    /// An analytic view: a named query over the plugin's entities (a dependency's exported ones, the directory, its other
    /// views) whose rows are <typeparamref name="T"/> (a <c>[View]</c> record), created by the platform in both engines
    /// and read with <c>ctx.Query&lt;T&gt;()</c> or <c>ctx.Analytics().Query&lt;T&gt;()</c>. Its query is LINQ over
    /// <see cref="Views.From{T}"/> ending in <c>select new T(..)</c>, each of the row's columns a column of a source (a
    /// bucket too: <c>Query.Month(o.PaidAt)</c>) or, grouped, an aggregate:
    /// <code>
    /// .View(from l in Views.From&lt;OrderLine&gt;()
    ///       join o in Views.From&lt;Order&gt;() on l.OrderId equals o.Id
    ///       where o.PaidAt &gt;= Query.DaysAgo(730)
    ///       select new Sales(Query.Day(o.PaidAt), o.CreatedBy, l.Amount))
    /// </code>
    /// Every row under a view passes its own entity's access rules and masks for whoever reads it.
    /// </summary>
    public Plugin View<T>(IQueryable<T> query) where T : class
    {
        _def.Views.Add((typeof(T), query.Expression));
        return this;
    }

    /// <summary>A layer of the Data Lake its tables belong to: <c>.Layer(new("raw") { Title = "As it arrived" })</c>.</summary>
    public Plugin Layer(Layer layer)
    {
        _def.Layers.Add(layer);
        return this;
    }

    /// <summary>A lake table of the plugin (a <c>[LakeTable]</c> record), in a layer the plugin declares.</summary>
    public Plugin LakeTable<T>() where T : class
    {
        _def.LakeTables.Add(typeof(T));
        return this;
    }

    /// <summary>Who reads a lake table of the plugin: the holders of these roles, every row of it (<c>ctx.Lake().Query&lt;T&gt;()</c>).</summary>
    public Plugin LakeRead<T>(params Role[] roles) where T : class
    {
        _def.LakeReads.Add((typeof(T), roles));
        return this;
    }

    /// <summary>The plugin's own system context (<c>ctx.AsSystem(..)</c>) reads the lake table too.</summary>
    public Plugin LakeReadAsSystem<T>() where T : class
    {
        _def.LakeSystemReads.Add(typeof(T));
        return this;
    }

    /// <summary>Other plugins that declare it (<c>.LakeDependency&lt;T&gt;()</c>) read the lake table, for callers holding one of its read roles.</summary>
    public Plugin LakeExport<T>() where T : class
    {
        _def.LakeExports.Add(typeof(T));
        return this;
    }

    /// <summary>Another plugin's exported lake table this plugin reads (<c>[LakeTable(Plugin = "retail", Name = "stock")]</c>).</summary>
    public Plugin LakeDependency<T>() where T : class
    {
        _def.LakeDependencies.Add(typeof(T));
        return this;
    }

    /// <summary>
    /// Other plugins that declare it (<c>.LakeDependency&lt;T&gt;()</c>) may copy the lake table into their own lake
    /// tables with a model, where their own roles decide who reads the copy (every row of it). Apart from
    /// <see cref="LakeExport{T}"/>; a later version that drops it stops further refreshes of the copies.
    /// </summary>
    public Plugin LakeExportToModels<T>() where T : class
    {
        _def.LakeModelExports.Add(typeof(T));
        return this;
    }

    /// <summary>
    /// How the platform computes the lake table <typeparamref name="T"/> of the plugin, when the plugin asks
    /// (<c>ctx.Lake().RefreshAsync&lt;T&gt;()</c>): LINQ over <see cref="LakeModel.From{T}"/> (a lake table, or an entity or
    /// an analytic view of the plugin; never both in one model) or <see cref="LakeModel.History{T}"/> ending in
    /// <c>select new T(..)</c>, each of the table's columns a column of a source (a bucket, a parsed text:
    /// <c>Query.Parse&lt;decimal&gt;(f.OnHand)</c>) or, grouped, an aggregate:
    /// <code>
    /// .LakeTable&lt;Stock&gt;()
    /// .Model(from f in LakeModel.From&lt;StockFileRow&gt;()
    ///        where f.Sku != ""
    ///        select new Stock(f.LoadedOn, f.Sku, Query.Parse&lt;decimal&gt;(f.OnHand)))
    /// </code>
    /// <paramref name="unmasked"/>: the masked columns of the entities under it go to the lake as they are (the lake has
    /// no masks; whoever reads the table reads every row of it).
    /// </summary>
    public Plugin Model<T>(IQueryable<T> query, bool unmasked = false) where T : class
    {
        _def.LakeModels.Add((typeof(T), query.Expression, unmasked));
        return this;
    }

    /// <summary>
    /// How the platform fills the entity stored in OLAP <typeparamref name="T"/> (<c>[Entity(Storage =
    /// EntityStorage.Olap, OlapKey = [..])]</c>) from the lake, when the plugin asks for a snapshot that loads it: LINQ
    /// over <see cref="LakeModel.From{T}"/> (a lake table the plugin's system context reads, or another plugin's whose
    /// owner lets models copy it) or <see cref="LakeModel.History{T}"/>, ending in <c>select new { .. }</c> whose members
    /// are the entity's properties (<c>Id</c> aside: it is made from the key; an optional one may be left out):
    /// <code>
    /// .Entity&lt;StoreMetric&gt;()
    /// .Load&lt;StoreMetric&gt;(from m in LakeModel.From&lt;StoreMetricsMart&gt;()
    ///                         where m.Day &gt;= Query.DaysAgo(400)
    ///                         select new { m.Day, Store = m.StoreId, m.Metric, Value = Query.Parse&lt;decimal&gt;(m.Value) })
    /// </code>
    /// </summary>
    public Plugin Load<T>(IQueryable query) where T : class
    {
        _def.Loads.Add((typeof(T), query.Expression));
        return this;
    }

    /// <summary>
    /// Who reads the history of an entity that keeps one (<c>[History("audit")]</c>): the holders of these roles, every row
    /// of it (<c>ctx.Lake().History&lt;T&gt;()</c>), without the entity's access rules or masks.
    /// </summary>
    public Plugin HistoryRead<T>(params Role[] roles) where T : class
    {
        _def.HistoryReads.Add((typeof(T), roles));
        return this;
    }

    /// <summary>The plugin's own system context (<c>ctx.AsSystem(..)</c>) reads the entity's history too (a model over it needs it).</summary>
    public Plugin HistoryReadAsSystem<T>() where T : class
    {
        _def.HistorySystemReads.Add(typeof(T));
        return this;
    }

    /// <summary>
    /// Other plugins that declare it (<c>.HistoryDependency&lt;T&gt;()</c>) read the entity's history, for callers holding
    /// one of its read roles.
    /// </summary>
    public Plugin HistoryExport<T>() where T : class
    {
        _def.HistoryExports.Add(typeof(T));
        return this;
    }

    /// <summary>
    /// Another plugin's exported history this plugin reads: its entity declared with its owner,
    /// <c>[Entity(Plugin = "helpdesk")]</c> (the type's name in snake_case is the entity's name there).
    /// </summary>
    public Plugin HistoryDependency<T>() where T : class
    {
        _def.LakeDependencies.Add(typeof(T));
        return this;
    }

    /// <summary>Declares a <c>[Message]</c> type (or enum) no endpoint uses.</summary>
    public Plugin Message<T>()
    {
        _def.Messages.Add(typeof(T));
        return this;
    }

    /// <summary>Adds an endpoint; its <c>Configure</c> runs now.</summary>
    public Plugin Endpoint<T>() where T : EndpointBase, new()
    {
        var endpoint = new T();
        var options = new EndpointOptions();
        endpoint.ConfigureCore(options);
        _def.Endpoints.Add(new EndpointRegistration(endpoint, EndpointName.Of(endpoint.RequestType), options));
        return this;
    }

    /// <summary>
    /// Adds a background job (<see cref="Job{T}"/>); its <c>Configure</c> runs now: <c>.Job&lt;RemindUnpaidJob&gt;()</c>.
    /// </summary>
    public Plugin Job<TJob>() where TJob : JobBase, new()
    {
        var job = new TJob();
        var options = new JobOptions();
        job.ConfigureCore(options);
        _def.Jobs.Add(new JobRegistration(job, options.NameValue ?? Jobs.Key(job.PayloadType.Name), options));
        return this;
    }

    /// <summary>
    /// How many runs of its jobs one process of the plugin runs at once, apart from its requests: 1 to 64, 16 by
    /// default. The platform never hands a process more runs at once, so a run's timeout starts when its handler does.
    /// </summary>
    public Plugin JobSlots(int n)
    {
        _def.JobSlots = n;
        return this;
    }

    /// <summary>A topic screens follow (<c>.Topic(Topics.Orders.Roles(Clerk))</c>); endpoints declare that they invalidate it.</summary>
    public Plugin Topic(Celobus.Sdk.Ui.TopicDecl topic)
    {
        _def.Topics.Add(topic);
        return this;
    }

    /// <summary>The plugin's UI: sidebar, screens, overlays, commands, home widgets (<see cref="Celobus.Sdk.Ui.PluginUi"/>).</summary>
    public Plugin Ui(Celobus.Sdk.Ui.PluginUi ui)
    {
        _def.Ui = ui;
        return this;
    }

    /// <summary>
    /// The run-change hook: the platform calls <paramref name="handler"/> when a run
    /// the plugin sees changes — a run of one of its agents, or one its code started — with where the run stands
    /// (<see cref="Sdk.RunChange"/>). A system request: no caller (<c>ctx.Caller.Kind</c> is <c>None</c>), so the
    /// handler writes with <c>ctx.AsSystem(..)</c> and reads the run with <c>ctx.Agents</c>.
    /// <para>
    /// Coalesced per run (one call per 250 ms window, then a trailing one with the latest state) and delivered at least
    /// once: an exception, the handler's 30 s timeout or a restart of the plugin makes the platform call again (after 1 s,
    /// 5 s, 30 s, 30 s; then it gives up and logs it). Handle it idempotently. The platform never waits for it to report a
    /// run.
    /// </para>
    /// <code>
    /// Plugin.Create("helpdesk").OnRunChange(async (ctx, change) =&gt;
    /// {
    ///     if (change.Ended) { var run = await ctx.Agents.Get(change.RunId); /* store run.Result, idempotently */ }
    /// });
    /// </code>
    /// </summary>
    public Plugin OnRunChange(Func<Ctx, RunChange, Task> handler)
    {
        _def.OnRunChange = handler;
        return this;
    }

    /// <summary>The definition errors the SDK can see (also checked by <c>celobus plugin check</c> and at publish).</summary>
    public IReadOnlyList<DescribeError> Check() => Descriptor.Build(_def).Errors;

    /// <summary>The descriptor JSON <c>--describe</c> prints.</summary>
    public string DescriptorJson() => Descriptor.Serialize(Descriptor.Build(_def).Json);

    /// <summary>
    /// With <c>--describe</c>: prints the descriptor on stdout and returns (exit 2 with the errors on stderr when the
    /// definition has any). Otherwise connects to <c>$CELOBUS_ADDR</c> and serves until SIGTERM or SIGINT.
    /// </summary>
    public Task Serve() => Serve(Environment.GetCommandLineArgs().Skip(1).ToArray());

    /// <inheritdoc cref="Serve()"/>
    public async Task Serve(string[] args, CancellationToken cancellationToken = default)
    {
        var (json, errors) = Descriptor.Build(_def);
        if (errors.Count > 0)
        {
            foreach (var e in errors) Console.Error.WriteLine(e);
            Environment.Exit(2);
        }
        if (args.Contains("--describe"))
        {
            Console.Out.WriteLine(Descriptor.Serialize(json));
            Console.Out.Flush();
            return;
        }

        var addr = Environment.GetEnvironmentVariable("CELOBUS_ADDR");
        if (string.IsNullOrEmpty(addr))
        {
            Console.Error.WriteLine("error: CELOBUS_ADDR is not set (the platform sets it; run with --describe to print the descriptor)");
            Environment.Exit(1);
        }

        using var stop = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        void OnSignal(PosixSignalContext c)
        {
            c.Cancel = true;
            stop.Cancel();
        }
        using var term = PosixSignalRegistration.Create(PosixSignal.SIGTERM, OnSignal);
        using var interrupt = PosixSignalRegistration.Create(PosixSignal.SIGINT, OnSignal);
        await new Runtime(_def).Run(addr!, stop.Token);
    }

    internal static string AssemblyVersion(Assembly? assembly)
    {
        var a = assembly ?? Assembly.GetEntryAssembly();
        var v = a?.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion
                ?? a?.GetName().Version?.ToString(3)
                ?? "0.0.0";
        var plus = v.IndexOf('+');
        return plus >= 0 ? v[..plus] : v;
    }
}
