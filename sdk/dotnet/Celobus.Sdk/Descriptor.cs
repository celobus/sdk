// The descriptor (schema 1) that `--describe` prints, and the definition checks the SDK can make.
// The CLI and the platform validate it again.
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;

namespace Celobus.Sdk;

/// <summary>A definition error: <c>error[E_CODE] at &lt;path&gt;: &lt;message&gt;</c>.</summary>
public sealed record DescribeError(string Code, string Path, string Message)
{
    public override string ToString() => $"error[{Code}] at {Path}: {Message}";
}

internal static partial class Descriptor
{
    private static readonly HashSet<string> ReservedIds =
    [
        "platform", "celobus", "core", "system", "sys", "workspace", "admin", "api", "app", "auth", "ui", "sdk", "cell",
        "public", "internal", "mcp",
    ];

    private static readonly string[] DefaultLabels = ["name", "title", "number"];

    public static (JsonObject Json, IReadOnlyList<DescribeError> Errors) Build(PluginDefinition p)
    {
        var errors = new List<DescribeError>();
        void Error(string code, string path, string message) => errors.Add(new DescribeError(code, path, message));

        // --- plugin
        if (!PluginId().IsMatch(p.Id)) Error("E_PLUGIN_ID", "id", $"`{p.Id}` must match ^[a-z][a-z0-9]{{1,15}}$");
        else if (ReservedIds.Contains(p.Id) || p.Id.StartsWith("pg", StringComparison.Ordinal))
            Error("E_PLUGIN_ID_RESERVED", "id", $"`{p.Id}` is reserved");
        if (!SemVer().IsMatch(p.Version)) Error("E_PLUGIN_VERSION", "version", $"`{p.Version}` is not a semantic version");

        // --- identity
        var roles = new JsonArray();
        var roleNames = new HashSet<string>();
        for (var i = 0; i < p.Roles.Count; i++)
        {
            var r = p.Roles[i];
            if (!RoleName().IsMatch(r.Name)) Error("E_ROLE_NAME", $"roles[{i}].name", $"`{r.Name}` must match ^[a-z][a-z0-9_]{{1,31}}$");
            if (!roleNames.Add(r.Name)) Error("E_ROLE_DUP", $"roles[{i}].name", $"role `{r.Name}` is declared twice");
            roles.Add(new JsonObject
            {
                ["name"] = r.Name,
                ["title"] = r.Title,
                ["description"] = r.Description,
                ["strict"] = r.Strict,
                ["default_for"] = r.DefaultFor.ToString().ToLowerInvariant(),
            });
        }

        var scopes = new JsonArray();
        var scopeNames = new HashSet<string>();
        for (var i = 0; i < p.Scopes.Count; i++)
        {
            var s = p.Scopes[i];
            if (!ScopeName().IsMatch(s.Name)) Error("E_SCOPE_NAME", $"scopes[{i}].name", $"`{s.Name}` must match ^[a-z][a-z0-9_.]{{0,62}}$");
            if (!scopeNames.Add(s.Name)) Error("E_SCOPE_DUP", $"scopes[{i}].name", $"scope `{s.Name}` is declared twice");
            scopes.Add(new JsonObject
            {
                ["name"] = s.Name,
                ["title"] = s.Title,
                ["description"] = s.Description,
                ["sensitive"] = s.Sensitive,
            });
        }

        var capabilities = new JsonArray();
        for (var i = 0; i < p.Capabilities.Count; i++)
        {
            var (c, reason) = p.Capabilities[i];
            if (!Capability.Known.Contains(c.Name)) Error("E_CAPABILITY_UNKNOWN", $"capabilities[{i}].name", $"unknown capability `{c.Name}`");
            if (string.IsNullOrWhiteSpace(reason)) Error("E_CAPABILITY_REASON", $"capabilities[{i}].reason", $"`{c.Name}` needs a reason");
            var capability = new JsonObject { ["name"] = c.Name, ["reason"] = reason };
            // the scope of authz:bypass, written only when declared
            if (c.Name == "authz:bypass" && p.Bypass.Count > 0)
                capability["on"] = new JsonArray([.. p.Bypass.Select(b => (JsonNode)new JsonObject
                {
                    ["entity"] = Names.Snake(b.Entity.Name),
                    ["ops"] = Strings(b.Ops),
                })]);
            capabilities.Add(capability);
        }
        if (p.Bypass.Count > 0 && p.Capabilities.All(x => x.Capability.Name != "authz:bypass"))
            Error("E_CAPABILITY_SCOPE", "capabilities", "a bypass scope needs the capability authz:bypass");

        var agents = new JsonArray();
        foreach (var a in p.Agents)
        {
            var agent = new JsonObject { ["name"] = a.Name, ["roles"] = Strings(a.RoleList) };
            // each left out when not declared, so a plugin without them describes itself as before
            if (a.WorkspaceList.Count > 0) agent["workspace"] = Strings(a.WorkspaceList);
            if (a.DelegateList.Count > 0) agent["delegates"] = Strings(a.DelegateList);
            if (a.ConnectionName.Length > 0) agent["connection"] = a.ConnectionName;
            if (a.HasSshKey) agent["ssh_key"] = true;
            if (a.IsAdmin) agent["admin"] = true;
            if (a.TitleText.Length > 0) agent["title"] = a.TitleText;
            if (a.Description.Length > 0) agent["description"] = a.Description;
            if (a.IconName.Length > 0) agent["icon"] = a.IconName;
            if (a.EngineValue is { } engine) agent["engine"] = engine.ToString().ToLowerInvariant();
            if (a.ModelName.Length > 0) agent["model"] = a.ModelName;
            if (a.IsSession) agent["session"] = true;
            if (a.TimeoutValue is { } timeout) agent["timeout_s"] = (long)timeout.TotalSeconds;
            if (a.SkillList.Count > 0) agent["skills"] = Strings(a.SkillList);
            if (a.TopicName.Length > 0)
            {
                if (p.Topics.All(t => t.Name != a.TopicName))
                    Error("E_AGENT_TOPIC_UNKNOWN", $"agents.{a.Name}.topic", $"topic `{a.TopicName}` is not declared");
                agent["topic"] = a.TopicName;
            }
            agents.Add(agent);
        }

        var dependencies = new JsonArray();
        var depNames = new HashSet<string>();
        for (var i = 0; i < p.Dependencies.Count; i++)
        {
            var dep = p.Dependencies[i];
            if (dep.Plugin == p.Id) Error("E_DEP_SELF", $"dependencies[{i}].plugin", "a plugin cannot depend on itself");
            if (!depNames.Add(dep.Plugin)) Error("E_DEP_DUP", $"dependencies[{i}].plugin", $"`{dep.Plugin}` is a dependency twice");
            var dependency = new JsonObject { ["plugin"] = dep.Plugin, ["calls"] = Strings(dep.Calls) };
            // written only when declared, so a plugin that reads none of its dependency's entities describes as before
            if (dep.Reads.Count > 0)
                dependency["reads"] = new JsonArray([.. dep.Reads.Select(r => (JsonNode)new JsonObject
                {
                    ["entity"] = r.Entity,
                    ["fields"] = new JsonArray([.. r.Fields.Select(f => (JsonNode)new JsonObject { ["name"] = f.Name, ["type"] = f.Type })]),
                })]);
            dependencies.Add(dependency);
        }

        // --- endpoints, and the messages they use
        var walk = new Walk();
        var endpoints = new JsonArray();
        var endpointNames = new HashSet<string>();
        for (var i = 0; i < p.Endpoints.Count; i++)
        {
            var e = p.Endpoints[i];
            var path = $"endpoints[{i}]";
            var req = e.Endpoint.RequestType;
            var res = e.Endpoint.ResponseType;
            if (!req.Name.EndsWith("Request", StringComparison.Ordinal) || req.Name == "Request")
                Error("E_ENDPOINT_INPUT_NAME", $"{path}.input", $"the request type `{req.Name}` of {e.Endpoint.GetType().Name} must be named <Name>Request");
            else if (!endpointNames.Add(e.Name)) Error("E_ENDPOINT_DUP", $"{path}.name", $"endpoint `{e.Name}` is declared twice");
            foreach (var (t, side) in new[] { (req, "input"), (res, "output") })
                if (!Schema.IsMessage(t)) Error("E_ENDPOINT_MESSAGE", $"{path}.{side}", $"`{Schema.Display(t)}` is not a [Message] or [Entity] type");
                else walk.Message(t);

            var o = e.Options;
            for (var j = 0; j < o.RoleList.Count; j++)
            {
                var r = o.RoleList[j];
                if (!r.IsPlatform && !roleNames.Contains(r.Name))
                    Error("E_ENDPOINT_ROLE_UNKNOWN", $"{path}.roles[{j}]", $"role `{r.Name}` is not declared (.Role(..)) nor workspace.*");
            }
            var endpointScopes = o.EffectiveScopes;
            for (var j = 0; j < endpointScopes.Count; j++)
                if (endpointScopes[j] is not ("read" or "write") && !scopeNames.Contains(endpointScopes[j]))
                    Error("E_ENDPOINT_SCOPE_UNKNOWN", $"{path}.scopes[{j}]", $"scope `{endpointScopes[j]}` is not declared (.Scope(..))");
            var timeoutMs = (long)Math.Round(o.TimeoutValue.TotalMilliseconds);
            if (timeoutMs is < 1 or > 600_000)
                Error("E_ENDPOINT_TIMEOUT_RANGE", $"{path}.timeout_ms", $"{timeoutMs} ms is outside 1 ms … 600 s");

            var endpoint = new JsonObject
            {
                ["name"] = e.Name,
                ["input"] = req.Name,
                ["output"] = res.Name,
                ["description"] = o.Description,
                ["auth"] = o.AuthKind.ToString().ToLowerInvariant(),
                ["roles"] = Strings(o.RoleList.Select(r => r.Name)),
                ["scopes"] = Strings(endpointScopes),
                ["idempotency"] = o.IdempotencyMode.ToString().ToLowerInvariant(),
                ["timeout_ms"] = timeoutMs,
            };
            // written only when declared, so endpoints without a UI describe as before
            if (o.IsUi)
            {
                if (o.IsReadOnly) endpoint["read_only"] = true;
                endpoint["ui"] = true;
            }
            if (o.InvalidationList.Count > 0)
                endpoint["invalidates"] = new JsonArray([.. o.InvalidationList.Select(v => (JsonNode)v.ToJson())]);
            endpoints.Add(endpoint);
        }

        // then the entities' messages, then the extra messages
        foreach (var t in p.Entities)
            if (Schema.IsForeignEntity(t))
                Error("E_ENT_NAME", $"entities.{t.Name}", $"`{t.Name}` is an entity of {Schema.ForeignPlugin(t)}: read it with ctx.Query<{t.Name}>(), do not declare it");
            else if (!Schema.IsEntity(t)) Error("E_ENT_NAME", $"entities.{t.Name}", $"`{t.Name}` has no [Entity] attribute");
            else walk.Message(t);
        foreach (var t in p.Messages)
            if (t.IsEnum) walk.Enum(t);
            else if (!Schema.IsMessage(t)) Error("E_MSG_NAME", $"messages.{t.Name}", $"`{t.Name}` has no [Message] attribute");
            else walk.Message(t);
        // then the jobs' payloads
        for (var i = 0; i < p.Jobs.Count; i++)
            if (Schema.IsMessage(p.Jobs[i].Payload)) walk.Message(p.Jobs[i].Payload);
        // then the topics' keys
        for (var i = 0; i < p.Topics.Count; i++)
            if (!Schema.IsMessage(p.Topics[i].KeyType))
                Error("E_MSG_NAME", $"topics[{i}].key", $"`{p.Topics[i].KeyType.Name}` has no [Message] attribute");
            else walk.Message(p.Topics[i].KeyType);
        // then the views' rows
        for (var i = 0; i < p.Views.Count; i++)
            if (!Schema.IsView(p.Views[i].Row))
                Error("E_VIEW_DEFINE", $"views[{i}]", $"`{p.Views[i].Row.Name}` has no [View] attribute");
            else walk.Message(p.Views[i].Row);
        // then the lake tables' rows (another plugin's table is not a message of this plugin)
        foreach (var t in p.LakeTables)
            if (Schema.IsLakeTable(t) && Schema.LakeOwner(t) is null) walk.Message(t);
        // then the settings message
        if (p.Settings is { } settingsDecl)
            if (!Schema.IsMessage(settingsDecl.Type))
                Error("E_SETTINGS_MESSAGE", "settings", $"`{settingsDecl.Type.Name}` has no [Message(Kind = MessageKind.Settings)] attribute");
            else walk.Message(settingsDecl.Type);

        // --- messages and enums
        var messages = new JsonArray();
        var messageNames = new Dictionary<string, Type>();
        foreach (var t in walk.Messages)
        {
            var schema = MessageSchema.For(t);
            if (messageNames.TryGetValue(schema.Name, out var other) && other != t)
                Error("E_MSG_DUP", $"messages.{schema.Name}", $"{t.FullName} and {other.FullName} are both named {schema.Name}");
            messageNames.TryAdd(schema.Name, t);
            messages.Add(new JsonObject
            {
                ["name"] = schema.Name,
                ["kind"] = schema.Kind.ToString().ToLowerInvariant(),
                ["description"] = schema.Description,
                ["fields"] = Fields(schema, p.Id, $"messages.{schema.Name}", Error),
            });
        }

        var enums = new JsonArray();
        var enumNames = new Dictionary<string, Type>();
        foreach (var t in walk.Enums)
        {
            if ((enumNames.TryGetValue(t.Name, out var other) || messageNames.TryGetValue(t.Name, out other)) && other != t)
                Error("E_ENUM_DUP", $"enums.{t.Name}", $"{t.FullName} and {other.FullName} are both named {t.Name}");
            enumNames.TryAdd(t.Name, t);
            enums.Add(new JsonObject { ["name"] = t.Name, ["values"] = Strings(Schema.EnumValues(t)) });
        }

        // --- entities: the registered ones, then the other entity types the messages use
        var entities = new JsonArray();
        var entityTypes = p.Entities.Where(Schema.IsEntity).Concat(walk.Messages.Where(Schema.IsEntity)).Distinct();
        var entityNames = new HashSet<string>();
        foreach (var t in entityTypes)
        {
            var schema = MessageSchema.For(t);
            var a = schema.Entity!;
            var name = Names.Snake(t.Name);
            var path = $"entities.{name}";
            if (!entityNames.Add(name)) Error("E_ENT_DUP", path, $"entity `{name}` is declared twice");
            if (name.EndsWith("_with_deleted", StringComparison.Ordinal))
                Error("E_ENT_NAME_RESERVED", path, "an entity name may not end in _with_deleted");
            var id = schema.Fields.FirstOrDefault(f => f.Name == "id");
            if (id is null || id.Kind != FieldKind.Ref || id.Repeated || id.ElementType.GetGenericArguments()[0] != t)
                Error("E_ENT_KEY", $"{path}.id", $"`{t.Name}` needs a field `Id` of type Id<{t.Name}>");
            string label;
            if (a.Label is { } l)
            {
                label = Names.Snake(l);
                if (schema.Fields.All(f => f.Name != label)) Error("E_ENT_LABEL", $"{path}.label", $"`{l}` is not a field of {t.Name}");
            }
            else label = DefaultLabels.FirstOrDefault(d => schema.Fields.Any(f => f.Name == d)) ?? "id";
            if (a.AppendOnly && a.Version) Error("E_ENT_APPEND_ONLY_VERSION", path, "an append-only entity has no version");
            if (a.AppendOnly && a.SoftDelete) Error("E_ENT_SOFT_DELETE_APPEND_ONLY", path, "an append-only entity cannot be soft-deleted");
            Columns(schema, path, Error, p.Id, a.AppendOnly);
            var entity = new JsonObject
            {
                ["name"] = name,
                ["message"] = schema.Name,
                ["key"] = a.Key switch { KeyType.I64 => "i64", KeyType.String => "string", _ => "uuid" },
                ["label"] = label,
                ["soft_delete"] = a.SoftDelete,
                ["version"] = a.Version,
                ["append_only"] = a.AppendOnly,
                ["fields"] = Fields(schema, p.Id, path, null, columns: true),
            };
            // written only when declared, so entities without them describe as before
            var indexes = new JsonArray();
            foreach (var ix in System.Reflection.CustomAttributeExtensions.GetCustomAttributes<IndexAttribute>(t, inherit: false))
            {
                var columns = new JsonArray();
                foreach (var c in ix.Columns)
                {
                    var parts = c.Split(' ', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
                    var column = new JsonObject { ["name"] = Names.Snake(parts.Length > 0 ? parts[0] : "") };
                    if (parts.Length > 1 && parts[1].Equals("desc", StringComparison.OrdinalIgnoreCase)) column["desc"] = true;
                    columns.Add(column);
                }
                var index = new JsonObject { ["columns"] = columns };
                if (ix.Unique) index["unique"] = true;
                indexes.Add(index);
            }
            if (indexes.Count > 0) entity["indexes"] = indexes;
            // the access rules, written only when declared
            if (p.Accesses.FirstOrDefault(x => x.Entity == t).Rules is { } rules) entity["access"] = rules.DeepClone();
            History(p, t, schema, entity, roleNames, path, Error);
            Olap(p, t, a, schema, entity, path, Error);
            entities.Add(entity);
        }
        foreach (var (what, list) in new[]
                 {
                     ("HistoryRead", p.HistoryReads.Select(x => x.Entity)), ("HistoryReadAsSystem", p.HistorySystemReads),
                     ("HistoryExport", p.HistoryExports),
                 })
            foreach (var t in list.Distinct())
                if (!Schema.IsEntity(t) || System.Reflection.CustomAttributeExtensions.GetCustomAttribute<HistoryAttribute>(t, false) is null)
                    Error("E_HISTORY_DECLARE", $"entities.{Names.Snake(t.Name)}.history",
                        $".{what}<{t.Name}>() names an entity that keeps no history: declare [History(\"<layer>\")] on it");
        foreach (var (t, _) in p.Accesses)
            if (!p.Entities.Contains(t))
                Error("E_ACCESS_ENTITY", $"access.{t.Name}", $"access rules for `{t.Name}`, which is not an entity of the plugin (.Entity<..>())");

        var json = new JsonObject
        {
            ["schema"] = 1,
            ["id"] = p.Id,
            ["version"] = p.Version,
            ["title"] = p.Title ?? Names.Humanize(p.Id),
            ["description"] = p.Description,
            ["roles"] = roles,
            ["scopes"] = scopes,
            ["capabilities"] = capabilities,
            ["permissions"] = Strings(p.Permissions),
            ["agents"] = agents,
            ["dependencies"] = dependencies,
            ["enums"] = enums,
            ["messages"] = messages,
            ["entities"] = entities,
            ["endpoints"] = endpoints,
        };
        if (p.Connections.Count > 0)
            json["connections"] = new JsonArray([.. p.Connections.Select(c =>
            {
                var connection = new JsonObject { ["name"] = c.Name, ["kind"] = c.Kind switch { ConnectionKind.Llm => "llm", ConnectionKind.Stt => "stt", _ => "maps" } };
                if (c.TitleText.Length > 0) connection["title"] = c.TitleText;
                if (c.Description.Length > 0) connection["description"] = c.Description;
                return (JsonNode)connection;
            })]);
        var (layers, lakeTables) = Lake(p, roleNames, entityNames, dependencies, Error);
        if (layers.Count > 0) json["layers"] = layers;
        if (lakeTables.Count > 0) json["lake_tables"] = lakeTables;
        var lakeModels = LakeModels(p, Error);
        if (lakeModels.Count > 0) json["lake_models"] = lakeModels;
        var loads = Loads(p, Error);
        if (loads.Count > 0) json["loads"] = loads;
        if (p.Topics.Count > 0) json["topics"] = new JsonArray([.. p.Topics.Select(t => (JsonNode)t.ToJson())]);
        if (p.Ui is not null)
        {
            var ui = p.Ui.Build();
            // upload inputs carry their file field's options, so a screen checks a file before uploading it
            FillUploads(ui, endpoints, messages);
            json["ui"] = ui;
        }

        // notification kinds: written only when declared, every member of each; they need push:send
        if (p.Notifications.Count > 0 && p.Capabilities.All(x => x.Capability.Name != "push:send"))
            Error("E_NOTIFY_CAPABILITY", "notifications", "notification kinds need the capability push:send");
        var kinds = new JsonArray();
        var kindNames = new HashSet<string>();
        for (var i = 0; i < p.Notifications.Count; i++)
        {
            var k = p.Notifications[i];
            if (!KindName().IsMatch(k.Name)) Error("E_NOTIFY_KIND_NAME", $"notifications[{i}]", $"`{k.Name}` must match ^[a-z][a-z0-9_]{{0,31}}$");
            else if (!kindNames.Add(k.Name)) Error("E_NOTIFY_KIND_NAME", $"notifications[{i}]", $"notification kind `{k.Name}` is declared twice");
            kinds.Add(new JsonObject
            {
                ["name"] = k.Name,
                ["title"] = k.Title,
                ["description"] = k.Description,
                ["tab"] = k.Tab == NotificationTab.Watching ? "watching" : "for_you",
                ["digest"] = k.Digest,
            });
        }
        if (kinds.Count > 0) json["notifications"] = kinds;
        // what the plugin asks of agent runs: written only when it handles the run-change hook
        if (p.OnRunChange is not null) json["runs"] = new JsonObject { ["on_change"] = true };
        // the settings: their message, whether the plugin checks a change, and their form (laid out and completed, or derived)
        if (p.Settings is { } decl && Schema.IsMessage(decl.Type))
        {
            var schema = MessageSchema.For(decl.Type);
            if (schema.Kind != MessageKind.Settings)
                Error("E_SETTINGS_MESSAGE", "settings", $"{schema.Name} is not a message of kind settings: declare it with [Message(Kind = MessageKind.Settings)]");
            var fields = messages.FirstOrDefault(m => (string?)m!["name"] == schema.Name)?["fields"]?.AsArray() ?? [];
            var sections = decl.Form is null
                ? SettingsDescribe.DeriveForm(fields, enums)
                : SettingsDescribe.CompleteForm(decl.Form(), fields, enums);
            json["settings"] = new JsonObject
            {
                ["message"] = schema.Name,
                ["on_change"] = decl.OnChange is not null,
                ["form"] = new JsonObject { ["sections"] = sections },
            };
        }
        // the options of a setting and the secret type belong to the settings message; its fields and form
        SettingsDescribe.Errors(messages, enums, json["settings"] as JsonObject, Error);
        // the background jobs: written only when declared
        var jobs = Jobs(p, Error);
        if (jobs.Count > 0) json["jobs"] = jobs;
        // the analytic views: written only when declared
        var views = Views(p, entityNames, Error);
        if (views.Count > 0) json["views"] = views;
        return (json, errors);
    }

    /// <summary>
    /// The <c>views</c> entries: each view's query tree as the descriptor writes it (lowercase operators, buckets,
    /// joins and functions; values as JSON), its columns in the order of its row's fields. The platform checks the rest
    /// (<c>E_VIEW_*</c>, <c>celobus plugin check</c>).
    /// </summary>
    /// <summary>
    /// The Data Lake's part of the descriptor: <c>layers</c>, <c>lake_tables</c> (each optional member only when set)
    /// and each dependency's <c>lake</c> (the tables of it read, on its entry, made after the declared ones when none).
    /// </summary>
    private static (JsonArray Layers, JsonArray Tables) Lake(PluginDefinition p, HashSet<string> roleNames,
        HashSet<string> entityNames, JsonArray dependencies, Action<string, string, string> error)
    {
        var layers = new JsonArray();
        var layerNames = new HashSet<string>();
        for (var i = 0; i < p.Layers.Count; i++)
        {
            var l = p.Layers[i];
            if (!LakeName().IsMatch(l.Name)) error("E_LAKE_LAYER_NAME", $"layers[{i}].name", $"`{l.Name}` is not a layer name (snake_case, at most 32 characters)");
            if (!layerNames.Add(l.Name)) error("E_LAKE_LAYER_DUP", $"layers[{i}]", $"layer `{l.Name}` is declared twice");
            var layer = new JsonObject { ["name"] = l.Name };
            if (l.Title.Length > 0) layer["title"] = l.Title;
            layers.Add(layer);
        }
        var viewNames = p.Views.Where(v => Schema.IsView(v.Row)).Select(v => Schema.ViewName(v.Row)).ToHashSet();
        var tables = new JsonArray();
        var names = new HashSet<string>();
        foreach (var t in p.LakeTables)
        {
            var path = $"lake_tables.{t.Name}";
            if (Schema.LakeOf(t) is not { } a)
            {
                error("E_LAKE_DEFINE", path, $"`{t.Name}` has no [LakeTable] attribute");
                continue;
            }
            if (a.Plugin is not null)
            {
                error("E_LAKE_DEFINE", path, $"`{t.Name}` is a lake table of {a.Plugin}: declare its read with .LakeDependency<{t.Name}>()");
                continue;
            }
            var name = Schema.LakeName(t);
            if (!LakeName().IsMatch(name) || name.Contains("__", StringComparison.Ordinal) || name.EndsWith('_'))
                error("E_LAKE_TABLE_NAME", path, $"`{name}` is not a lake table name (snake_case, at most 32 characters, no `__`, not ending in `_`)");
            if (!names.Add(name)) error("E_LAKE_TABLE_DUP", path, $"lake table `{name}` is declared twice");
            if (entityNames.Contains(name) || viewNames.Contains(name))
                error("E_LAKE_TABLE_CLASH", path, $"`{name}` is the name of an entity or a view of the plugin");
            if (a.Layer is null || !layerNames.Contains(a.Layer))
                error("E_LAKE_TABLE_LAYER", $"{path}.layer", $"`{a.Layer}` is not a layer the plugin declares (.Layer(..))");
            var schema = MessageSchema.For(t);
            JsonArray Columns(string[] list, string what, string code)
            {
                var out_ = new JsonArray();
                foreach (var c in list)
                {
                    var column = Names.Snake(c);
                    if (schema.Fields.All(f => f.Name != column)) error(code, $"{path}.{what}", $"`{c}` is not a field of {t.Name}");
                    else out_.Add(column);
                }
                return out_;
            }
            var table = new JsonObject { ["name"] = name, ["layer"] = a.Layer ?? "", ["message"] = schema.Name };
            if (schema.Description.Length > 0) table["description"] = schema.Description;
            if (Columns(a.Partition, "partition", "E_LAKE_PARTITION") is { Count: > 0 } partition) table["partition"] = partition;
            if (Columns(a.Key, "key", "E_LAKE_KEY") is { Count: > 0 } key) table["key"] = key;
            var read = new JsonArray();
            foreach (var r in p.LakeReads.Where(x => x.Table == t).SelectMany(x => x.Roles))
                if (r.IsPlatform || !roleNames.Contains(r.Name))
                    error("E_LAKE_ROLE", $"{path}.read", $"role `{r.Name}` is not a role of the plugin (.Role(..))");
                else if (read.All(x => (string?)x != r.Name)) read.Add(r.Name); // each once, where first named
            if (read.Count > 0) table["read"] = read;
            if (p.LakeSystemReads.Contains(t)) table["system_read"] = true;
            if (p.LakeExports.Contains(t)) table["export"] = true;
            if (p.LakeModelExports.Contains(t)) table["export_models"] = true;
            tables.Add(table);
        }
        foreach (var t in p.LakeReads.Select(x => x.Table).Concat(p.LakeSystemReads).Concat(p.LakeExports)
                     .Concat(p.LakeModelExports).Distinct())
            if (!p.LakeTables.Contains(t))
                error("E_LAKE_DEFINE", $"lake_tables.{t.Name}", $"`{t.Name}` is not a lake table of the plugin: declare it with .LakeTable<{t.Name}>() first");
        foreach (var t in p.LakeDependencies)
        {
            // another plugin's lake table, or its entity whose history is read (.HistoryDependency<T>())
            var owner = Schema.LakeOwner(t) ?? (Schema.IsLakeTable(t) ? null : Schema.ForeignPlugin(t));
            if (owner is null)
            {
                error("E_LAKE_DEFINE", $"dependencies.lake.{t.Name}",
                    $"`{t.Name}` is not another plugin's lake table or entity: .LakeDependency<T>() names one ([LakeTable(Plugin = .., Name = ..)]), .HistoryDependency<T>() one of [Entity(Plugin = ..)]");
                continue;
            }
            var dep = dependencies.Select(d => d!.AsObject()).FirstOrDefault(d => (string?)d["plugin"] == owner);
            if (dep is null)
            {
                dep = new JsonObject { ["plugin"] = owner, ["calls"] = new JsonArray() };
                dependencies.Add(dep);
            }
            if (dep["lake"] is not JsonArray lake) dep["lake"] = lake = new JsonArray();
            var table = Schema.IsLakeTable(t) ? Schema.LakeName(t) : Names.Snake(t.Name);
            if (lake.All(x => (string?)x != table)) lake.Add(table);
        }
        return (layers, tables);
    }

    private static JsonArray Views(PluginDefinition p, HashSet<string> entityNames, Action<string, string, string> error)
    {
        var views = new JsonArray();
        var names = new HashSet<string>();
        for (var i = 0; i < p.Views.Count; i++)
        {
            var (row, query) = p.Views[i];
            if (!Schema.IsView(row)) continue;
            var name = Schema.ViewName(row);
            var path = $"views[{i}]";
            if (!SqlName().IsMatch(name)) error("E_VIEW_NAME", path, $"`{name}` must match ^[a-z][a-z0-9_]{{0,62}}$");
            if (!names.Add(name)) error("E_VIEW_DUP", path, $"view `{name}` is declared twice");
            if (entityNames.Contains(name)) error("E_VIEW_CLASH", path, $"`{name}` is the name of an entity of the plugin");
            Celobus.V1.Query tree;
            List<(string Name, Celobus.V1.QueryTerm Term)> columns;
            try
            {
                (tree, columns) = Translator.TranslateView(query, row);
            }
            catch (NotSupportedException e)
            {
                error("E_VIEW_DEFINE", path, e.Message);
                continue;
            }
            var schema = MessageSchema.For(row);
            var out_ = new JsonArray();
            foreach (var c in columns)
                if (schema.Fields.All(f => f.Name != c.Name))
                    error("E_VIEW_DEFINE", path, $"{c.Name} is not a column of {row.Name}");
            foreach (var f in schema.Fields)
            {
                var c = columns.FirstOrDefault(x => x.Name == f.Name);
                if (c.Term is null)
                {
                    error("E_VIEW_DEFINE", path, $"the column {f.Name} is not given");
                    continue;
                }
                out_.Add(new JsonObject { ["name"] = f.Name, ["value"] = ViewTerm(c.Term) });
            }
            var view = new JsonObject
            {
                ["name"] = name,
                ["message"] = schema.Name,
                ["description"] = schema.Description,
                ["sources"] = ViewSources(tree),
            };
            if (tree.Filter is not null) view["filter"] = ViewExpr(tree.Filter);
            if (tree.GroupBy.Count > 0) view["group_by"] = new JsonArray([.. tree.GroupBy.Select(c => (JsonNode)ViewColumn(c))]);
            if (tree.Having is not null) view["having"] = ViewExpr(tree.Having);
            view["columns"] = out_;
            views.Add(view);
        }
        return views;
    }

    private static JsonArray ViewSources(Celobus.V1.Query tree) => new([.. tree.Sources.Select(s =>
    {
        var src = new JsonObject { ["entity"] = s.Entity, ["alias"] = s.Alias };
        if (s.Join != Celobus.V1.QuerySource.Types.Join.Root) src["join"] = s.Join.ToString().ToLowerInvariant();
        if (s.Left is not null) src["left"] = ViewColumn(s.Left);
        if (s.Right is not null) src["right"] = ViewColumn(s.Right);
        if (s.History) src["history"] = true;
        if (s.EachDay) src["each_day"] = true;
        return (JsonNode)src;
    })]);

    private static JsonObject ViewColumn(Celobus.V1.QueryColumn c)
    {
        var o = new JsonObject { ["source"] = c.Source, ["field"] = c.Field };
        if (c.Bucket != Celobus.V1.QueryColumn.Types.Bucket.None) o["bucket"] = c.Bucket.ToString().ToLowerInvariant();
        // `PARSE_DECIMAL` on the wire, `decimal` in the descriptor
        if (c.Parse != Celobus.V1.QueryColumn.Types.Parse.None) o["parse"] = c.Parse.ToString().ToLowerInvariant();
        return o;
    }

    /// <summary>
    /// The <c>lake_models</c> entries, in the order declared: each model's query tree as a view's, its columns in the
    /// order of its table's fields. The platform checks the rest (<c>E_LAKE_MODEL_*</c>, <c>celobus plugin check</c>).
    /// </summary>
    private static JsonArray LakeModels(PluginDefinition p, Action<string, string, string> error)
    {
        var models = new JsonArray();
        for (var i = 0; i < p.LakeModels.Count; i++)
        {
            var (table, query, unmasked) = p.LakeModels[i];
            var path = $"lake_models[{i}]";
            if (!p.LakeTables.Contains(table) || Schema.LakeOf(table) is not { Plugin: null })
            {
                error("E_LAKE_MODEL_TABLE", path, $"`{table.Name}` is not a lake table of the plugin: declare it with .LakeTable<{table.Name}>() first");
                continue;
            }
            // lake tables (or a history) with entities or views in one model: the platform's own code for it
            var roots = Translator.Roots(query);
            if (roots.Any(Schema.IsLakeSource) && roots.Any(r => !Schema.IsLakeSource(r)))
            {
                error("E_LAKE_MODEL_MIXED", path,
                    $"the model of {Schema.LakeName(table)} reads lake tables ({string.Join(", ", roots.Where(Schema.IsLakeSource).Select(r => r.Name))}) "
                    + $"and entities or views ({string.Join(", ", roots.Where(r => !Schema.IsLakeSource(r)).Select(r => r.Name))}): a model reads one or the other");
                continue;
            }
            Celobus.V1.Query tree;
            List<(string Name, Celobus.V1.QueryTerm Term)> columns;
            try
            {
                (tree, columns) = Translator.TranslateView(query, table);
            }
            catch (NotSupportedException e)
            {
                error("E_LAKE_MODEL_DEFINE", path, e.Message);
                continue;
            }
            var schema = MessageSchema.For(table);
            var out_ = new JsonArray();
            foreach (var f in schema.Fields)
            {
                var c = columns.FirstOrDefault(x => x.Name == f.Name);
                if (c.Term is null) error("E_LAKE_MODEL_DEFINE", path, $"the column {f.Name} is not given");
                else out_.Add(new JsonObject { ["name"] = f.Name, ["value"] = ViewTerm(c.Term) });
            }
            var model = new JsonObject { ["table"] = Schema.LakeName(table), ["sources"] = ViewSources(tree) };
            if (tree.Filter is not null) model["filter"] = ViewExpr(tree.Filter);
            if (tree.GroupBy.Count > 0) model["group_by"] = new JsonArray([.. tree.GroupBy.Select(c => (JsonNode)ViewColumn(c))]);
            if (tree.Having is not null) model["having"] = ViewExpr(tree.Having);
            model["columns"] = out_;
            if (unmasked) model["unmasked"] = true;
            models.Add(model);
        }
        return models;
    }

    /// <summary>
    /// An entity's <c>history</c> (<c>[History("layer")]</c>: its layer, <c>all</c>, <c>unmasked</c>, who reads it) and,
    /// with <c>All</c>, the flag on every column it covers by the platform's rule: every column but <c>id</c>, the
    /// encrypted ones, the lists and messages, the history's own column names and, without <c>Unmasked</c>, the masked
    /// ones.
    /// </summary>
    private static void History(PluginDefinition p, Type t, MessageSchema schema, JsonObject entity, HashSet<string> roleNames,
        string path, Action<string, string, string> error)
    {
        var fields = entity["fields"]!.AsArray();
        var h = System.Reflection.CustomAttributeExtensions.GetCustomAttribute<HistoryAttribute>(t, false);
        if (h is null)
        {
            if (schema.Fields.FirstOrDefault(f => f.History && f.Name != "id") is { } kept)
                error("E_HISTORY_LAYER", $"{path}.fields.{kept.Name}.history",
                    $"{entity["name"]}.{kept.Name} is kept, but the entity keeps no history: declare [History(\"<layer>\")] on it");
            return;
        }
        if (h.Layer is null || p.Layers.All(l => l.Name != h.Layer))
            error("E_HISTORY_LAYER", $"{path}.history.layer", $"`{h.Layer}` is not a layer the plugin declares (.Layer(..))");
        var history = new JsonObject { ["layer"] = h.Layer ?? "" };
        if (h.All) history["all"] = true;
        if (h.Unmasked) history["unmasked"] = true;
        var read = new JsonArray();
        foreach (var r in p.HistoryReads.Where(x => x.Entity == t).SelectMany(x => x.Roles))
            if (r.IsPlatform || !roleNames.Contains(r.Name))
                error("E_HISTORY_ROLE", $"{path}.history.read", $"role `{r.Name}` is not a role of the plugin (.Role(..))");
            else if (read.All(x => (string?)x != r.Name)) read.Add(r.Name); // each once, where first named
        if (read.Count > 0) history["read"] = read;
        if (p.HistorySystemReads.Contains(t)) history["system_read"] = true;
        if (p.HistoryExports.Contains(t)) history["export"] = true;
        entity["history"] = history;
        if (!h.All) return;
        var masked = (entity["access"]?["masks"] as JsonArray ?? [])
            .Select(m => (string?)m?["field"]).OfType<string>().ToHashSet();
        foreach (var node in fields)
        {
            var f = node!.AsObject();
            var name = (string)f["name"]!;
            var type = (string)f["type"]!;
            var keepable = !(bool)f["repeated"]! && (LakeScalars.Contains(type) || type.StartsWith("enum:", StringComparison.Ordinal)
                                                       || type.StartsWith("ref:", StringComparison.Ordinal));
            if (name == "id" || f["encrypted"] is not null || Schema.HistoryColumns.Contains(name) || !keepable) continue;
            if (!h.Unmasked && masked.Contains(name)) continue;
            f["history"] = true;
        }
    }

    /// <summary>
    /// An entity stored in OLAP: <c>storage</c> and <c>olap_key</c> (each written only when declared), and the SDK's own
    /// checks of its declaration (<c>E_OLAP_KEY</c>, <c>E_OLAP_LOAD</c>); the platform checks the rest.
    /// </summary>
    private static void Olap(PluginDefinition p, Type t, EntityAttribute a, MessageSchema schema, JsonObject entity, string path,
        Action<string, string, string> error)
    {
        var olap = a.Storage == EntityStorage.Olap;
        if (olap) entity["storage"] = "olap";
        else if (a.Storage != EntityStorage.Default)
            error("E_OLAP_STORAGE", $"{path}.storage", $"`{a.Storage}` is not a storage");
        if (a.OlapKey.Length > 0)
        {
            if (!olap) error("E_OLAP_KEY", $"{path}.olap_key", "OlapKey keys an entity stored in OLAP: declare Storage = EntityStorage.Olap");
            var key = new JsonArray();
            foreach (var k in a.OlapKey)
            {
                var name = Names.Snake(k);
                if (schema.Fields.All(f => f.Name != name)) error("E_OLAP_KEY", $"{path}.olap_key", $"`{k}` is not a field of {t.Name}");
                key.Add(name);
            }
            entity["olap_key"] = key;
        }
        var loads = p.Loads.Count(l => l.Entity == t);
        if (olap && loads == 0)
            error("E_OLAP_LOAD", $"{path}.load", $"{t.Name} is stored in OLAP: its load fills it (.Load<{t.Name}>(..))");
        if (!olap && loads > 0)
            error("E_LOAD_ENTITY", $"{path}.load", $"{t.Name} is not stored in OLAP: only such an entity has a load");
    }

    /// <summary>
    /// The <c>loads</c> entries, in the order declared: each load's query tree as a view's, its columns in the order of
    /// its entity's fields (<c>id</c> aside; an optional one left out reads absent).
    /// </summary>
    private static JsonArray Loads(PluginDefinition p, Action<string, string, string> error)
    {
        var loads = new JsonArray();
        for (var i = 0; i < p.Loads.Count; i++)
        {
            var (entity, query) = p.Loads[i];
            var path = $"loads[{i}]";
            if (!Schema.IsEntity(entity) || !p.Entities.Contains(entity))
            {
                error("E_LOAD_ENTITY", path, $"`{entity.Name}` is not an entity of the plugin: declare it with .Entity<{entity.Name}>() first");
                continue;
            }
            Celobus.V1.Query tree;
            List<(string Name, Celobus.V1.QueryTerm Term)> columns;
            try
            {
                (tree, columns) = Translator.TranslateLoad(query);
            }
            catch (NotSupportedException e)
            {
                error("E_LOAD_DEFINE", path, e.Message);
                continue;
            }
            var schema = MessageSchema.For(entity);
            foreach (var c in columns)
                if (c.Name == "id") error("E_LOAD_COLUMN", path, "id is made from the key: a load does not give it");
                else if (schema.Fields.All(f => f.Name != c.Name)) error("E_LOAD_COLUMN", path, $"{c.Name} is not a field of {entity.Name}");
            var out_ = new JsonArray();
            foreach (var f in schema.Fields.Where(f => f.Name != "id"))
            {
                var c = columns.FirstOrDefault(x => x.Name == f.Name);
                if (c.Term is not null) out_.Add(new JsonObject { ["name"] = f.Name, ["value"] = ViewTerm(c.Term) });
                else if (!f.Optional) error("E_LOAD_COLUMN", path, $"the column {f.Name} is not given (a required field of {entity.Name})");
            }
            var load = new JsonObject { ["entity"] = Names.Snake(entity.Name), ["sources"] = ViewSources(tree) };
            if (tree.Filter is not null) load["filter"] = ViewExpr(tree.Filter);
            if (tree.GroupBy.Count > 0) load["group_by"] = new JsonArray([.. tree.GroupBy.Select(c => (JsonNode)ViewColumn(c))]);
            if (tree.Having is not null) load["having"] = ViewExpr(tree.Having);
            load["columns"] = out_;
            loads.Add(load);
        }
        return loads;
    }

    /// <summary>The scalar types a lake column holds.</summary>
    private static readonly HashSet<string> LakeScalars =
        ["string", "bool", "int32", "int64", "double", "decimal", "uuid", "timestamp", "date", "bytes", "json"];

    private static JsonObject ViewAggregate(Celobus.V1.QueryAggregate a)
    {
        var o = new JsonObject { ["fn"] = Names.Snake(a.Fn.ToString()) };
        if (a.Column is not null) o["column"] = ViewColumn(a.Column);
        return o;
    }

    private static JsonObject ViewTerm(Celobus.V1.QueryTerm t) => t.KindCase switch
    {
        Celobus.V1.QueryTerm.KindOneofCase.Column => new JsonObject { ["column"] = ViewColumn(t.Column) },
        Celobus.V1.QueryTerm.KindOneofCase.Aggregate => new JsonObject { ["aggregate"] = ViewAggregate(t.Aggregate) },
        _ => new JsonObject { ["days_ago"] = t.DaysAgo },
    };

    private static JsonNode? ViewValue(Celobus.V1.DbValue v) => v.KindCase switch
    {
        Celobus.V1.DbValue.KindOneofCase.StringValue => v.StringValue,
        Celobus.V1.DbValue.KindOneofCase.IntValue => v.IntValue,
        Celobus.V1.DbValue.KindOneofCase.DoubleValue => v.DoubleValue,
        Celobus.V1.DbValue.KindOneofCase.BoolValue => v.BoolValue,
        _ => null,
    };

    private static JsonObject ViewExpr(Celobus.V1.QueryExpr e)
    {
        switch (e.NodeCase)
        {
            case Celobus.V1.QueryExpr.NodeOneofCase.And:
                return new JsonObject { ["and"] = new JsonArray([.. e.And.Items.Select(x => (JsonNode)ViewExpr(x))]) };
            case Celobus.V1.QueryExpr.NodeOneofCase.Or:
                return new JsonObject { ["or"] = new JsonArray([.. e.Or.Items.Select(x => (JsonNode)ViewExpr(x))]) };
            case Celobus.V1.QueryExpr.NodeOneofCase.Not:
                return new JsonObject { ["not"] = ViewExpr(e.Not) };
            case Celobus.V1.QueryExpr.NodeOneofCase.Compare:
            {
                var c = e.Compare;
                var o = new JsonObject { ["left"] = ViewTerm(c.Left), ["op"] = Names.Snake(c.Op.ToString()) };
                if (c.Values.Count > 0) o["values"] = new JsonArray([.. c.Values.Select(ViewValue)]);
                if (c.Right is not null) o["right"] = ViewTerm(c.Right);
                return new JsonObject { ["compare"] = o };
            }
            default:
            {
                var x = e.Exists;
                var o = new JsonObject { ["entity"] = x.Source.Entity, ["alias"] = x.Source.Alias };
                if (x.Filter is not null) o["filter"] = ViewExpr(x.Filter);
                return new JsonObject { ["exists"] = o };
            }
        }
    }

    /// <summary>The <c>jobs</c> entries, each member written only when declared, and their rules (<c>E_JOB_*</c>).</summary>
    private static JsonArray Jobs(PluginDefinition p, Action<string, string, string> error)
    {
        var jobs = new JsonArray();
        var names = new HashSet<string>();
        var payloads = new HashSet<Type>();
        for (var i = 0; i < p.Jobs.Count; i++)
        {
            var j = p.Jobs[i];
            var o = j.Options;
            var path = $"jobs[{(JobPathName().IsMatch(j.Key) ? j.Key : i.ToString(System.Globalization.CultureInfo.InvariantCulture))}]";
            if (j.Key.Length > 32 || !SqlName().IsMatch(j.Key))
                error("E_JOB_NAME", $"{path}.name", $"\"{j.Key}\" is not a job name (snake_case, at most 32 characters: remind_unpaid)");
            if (!names.Add(j.Key)) error("E_JOB_DUP", path, $"job \"{j.Key}\" is declared twice");
            if (!Schema.IsMessage(j.Payload))
                error("E_JOB_PAYLOAD", $"{path}.payload", $"\"{j.Payload.Name}\" is not a [Message] type (declare the payload message: [Message(Kind = MessageKind.Job)])");
            else if (!payloads.Add(j.Payload))
                error("E_JOB_PAYLOAD_DUP", $"{path}.payload",
                    $"message \"{j.Payload.Name}\" is the payload of another job (a job is picked by its payload's type: give each job a message of its own)");
            if (o.CronValue.Length > 0)
            {
                if (Cron.Check(o.CronValue) is { } why)
                    error("E_JOB_CRON", $"{path}.cron",
                        $"\"{o.CronValue}\": {why} (five fields in UTC, minute hour day-of-month month day-of-week: 0 9 * * 1-5)");
                if (Schema.IsMessage(j.Payload) && RequiredWithoutDefault(j.Payload, []) is { } field)
                    error("E_JOB_SCHEDULE_PAYLOAD", $"{path}.payload",
                        $"a scheduled job runs with its payload's defaults, and {field} has none (make the field optional, or start the job by enqueueing it instead of a schedule)");
            }
            long? delayMs = null;
            if (o.RetryValue is { } retry)
            {
                delayMs = (long)Math.Floor(retry.Delay.TotalMilliseconds);
                if (retry.Retries is < 0 or > 20)
                    error("E_JOB_RETRY", $"{path}.retry.retries", $"{retry.Retries} retries; a job has 0 to 20");
                if (delayMs is < 0 or > 3_600_000)
                    error("E_JOB_RETRY", $"{path}.retry.delay_ms", $"a retry delay of {delayMs} ms; 0 to one hour (3600000 ms)");
            }
            long? timeoutMs = o.TimeoutValue is { } t ? (long)Math.Floor(t.TotalMilliseconds) : null;
            if (timeoutMs is < 1_000 or > 3_600_000)
                error("E_JOB_TIMEOUT", $"{path}.timeout_ms", $"a timeout of {timeoutMs} ms; a run's timeout is 1 s to 1 h");
            if (o.ConcurrencyValue is < 1 or > 64)
                error("E_JOB_CONCURRENCY", $"{path}.concurrency", $"a concurrency of {o.ConcurrencyValue}; a job runs 1 to 64 runs at once");

            var job = new JsonObject { ["name"] = j.Key, ["payload"] = j.Payload.Name };
            if (o.Description.Length > 0) job["description"] = o.Description;
            if (o.CronValue.Length > 0) job["cron"] = o.CronValue;
            if (o.IsSystem) job["as_system"] = true;
            if (o.RetryValue is { } r)
                job["retry"] = new JsonObject
                {
                    ["retries"] = r.Retries,
                    ["delay_ms"] = delayMs,
                    ["backoff"] = r.Backoff == Backoff.Fixed ? "fixed" : "exponential",
                };
            if (timeoutMs is { } ms) job["timeout_ms"] = ms;
            if (o.ConcurrencyValue is { } n) job["concurrency"] = n;
            jobs.Add(job);
        }
        if (p.JobSlots is < 1 or > PluginDefinition.MaxJobSlots)
            error("E_JOB_SLOTS", "job_slots", $"{p.JobSlots} job slots; a process runs 1 to {PluginDefinition.MaxJobSlots} runs at once");
        return jobs;
    }

    /// <summary>
    /// The first field of a payload that a run read from <c>{}</c> cannot fill (<c>RemindUnpaid.since</c>): a single
    /// timestamp or date that is not optional, directly or in a message field that is not optional.
    /// </summary>
    private static string? RequiredWithoutDefault(Type t, HashSet<Type> seen)
    {
        if (!seen.Add(t)) return null;
        var schema = MessageSchema.For(t);
        foreach (var f in schema.Fields)
        {
            if (f.Optional || f.Repeated) continue;
            if (f.Kind is FieldKind.Timestamp or FieldKind.Date) return $"{schema.Name}.{f.Name}";
            if (f.Kind == FieldKind.Message && RequiredWithoutDefault(f.ElementType, seen) is { } found) return found;
        }
        return null;
    }

    public static string Serialize(JsonObject json) => json.ToJsonString(new JsonSerializerOptions
    {
        WriteIndented = true,
        Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
    });

    // `columns`: an entity's fields, with their column modifiers (its message's copy has none)
    private static JsonArray Fields(MessageSchema schema, string pluginId, string path, Action<string, string, string>? error,
        bool columns = false)
    {
        var fields = new JsonArray();
        foreach (var f in schema.Fields)
        {
            if (f.Error != null) error?.Invoke("E_FIELD_TYPE", $"{path}.fields.{f.Name}", f.Error);
            var field = new JsonObject
            {
                ["name"] = f.Name,
                ["number"] = f.Number,
                ["type"] = f.DescriptorType(pluginId),
                ["repeated"] = f.Repeated,
                ["optional"] = f.Optional,
                ["description"] = f.Description,
            };
            if (columns && f.Unique) field["unique"] = true;
            if (columns && f.Indexed) field["index"] = true;
            if (columns && f.Encrypted) field["encrypted"] = true;
            if (columns && !string.IsNullOrEmpty(f.Searchable)) field["searchable"] = f.Searchable;
            if (columns && !string.IsNullOrEmpty(f.Nav)) field["nav"] = f.Nav;
            if (columns && !string.IsNullOrEmpty(f.Inverse)) field["inverse"] = f.Inverse;
            if (columns && !string.IsNullOrEmpty(f.OnDelete)) field["on_delete"] = f.OnDelete;
            if (columns && f.History && f.Name != "id") field["history"] = true;
            // a file field's options, each written only when declared (on the entity's fields and its message's alike)
            if (f.MaxSize is { } maxSize)
            {
                if (FileSize.Parse(maxSize) is { } bytes) field["max_size"] = bytes;
                else error?.Invoke("E_FILE_OPTION", $"{path}.fields.{f.Name}.max_size",
                    $"`{maxSize}` is not a size: bytes (1048576) or a number and a unit (\"1 GiB\", \"500 MB\"; KiB, MiB, GiB, TiB, KB, MB, GB, TB)");
            }
            if (f.Accept.Count > 0) field["accept"] = Strings(f.Accept);
            if ((f.MaxSize is not null || f.Accept.Count > 0) && f.Kind != FieldKind.File)
                error?.Invoke("E_FILE_OPTION", $"{path}.fields.{f.Name}",
                    $"{f.Name} is not a file field: [MaxSize] and [Accept] belong to a File, File? or List<File>");
            // a setting's options ([Label], [Range], a default, ..), each written only when declared
            SettingsDescribe.Keys(f, field, $"{path}.fields.{f.Name}", error);
            fields.Add(field);
        }
        return fields;
    }

    /// <summary>The text search languages of a searchable column (PostgreSQL's configurations).</summary>
    internal static readonly HashSet<string> SearchLanguages =
    [
        "simple", "arabic", "armenian", "basque", "catalan", "danish", "dutch", "english", "estonian", "finnish", "french",
        "german", "greek", "hindi", "hungarian", "indonesian", "irish", "italian", "lithuanian", "nepali", "norwegian",
        "portuguese", "romanian", "russian", "serbian", "spanish", "swedish", "tamil", "turkish", "yiddish",
    ];

    // What the SDK sees of the searchable columns and the navigations of an entity's fields; duplicates of names and
    // inverse targets are checked by `celobus plugin check` (crates/descriptor).
    private static void Columns(MessageSchema schema, string path, Action<string, string, string> error, string pluginId,
        bool appendOnly)
    {
        var searchable = 0;
        foreach (var f in schema.Fields)
        {
            var fp = $"{path}.fields.{f.Name}";
            if (f.Encrypted && f.Kind == FieldKind.File)
                error("E_FILE_ENCRYPTED", fp, $"{f.Name} holds files: a file column is never encrypted (its bytes are read directly by their URLs)");
            if (f.Searchable is { } lang)
            {
                if (++searchable == 9) error("E_SEARCH_LIMIT", fp, "an entity has at most 8 searchable columns");
                if (f.Kind != FieldKind.String || f.Repeated || f.Name == "id")
                    error("E_SEARCH_TYPE", fp, $"{f.Name} is not a text column: only a single string column is searchable");
                if (!SearchLanguages.Contains(lang)) error("E_SEARCH_LANGUAGE", $"{fp}.searchable", $"`{lang}` is not a text search language");
            }
            foreach (var (what, name) in new[] { ("nav", f.Nav), ("inverse", f.Inverse) })
            {
                if (name is null) continue;
                if (f.Kind != FieldKind.Ref || f.Repeated || f.Name == "id")
                    error("E_REF_NAME", $"{fp}.{what}", $"{f.Name} is not a ref column: only a single ref has a {what}");
                else if (!SqlName().IsMatch(name)) error("E_REF_NAME", $"{fp}.{what}", $"`{name}` must match ^[a-z][a-z0-9_]{{0,62}}$");
            }
            if (f.OnDelete is { } od)
            {
                var dp = $"{fp}.on_delete";
                if (f.Kind != FieldKind.Ref || f.Repeated || f.Name == "id")
                {
                    error("E_REF_ON_DELETE", dp, $"{f.Name} is not a ref column: only a single ref has an OnDelete");
                    continue;
                }
                var target = f.DescriptorType(pluginId)["ref:".Length..].Split('.')[0];
                if (target == pluginId && od == "keep")
                    error("E_REF_ON_DELETE", dp, "Keep is for a ref to another plugin or the directory: Restrict, Cascade, SetNull or NoAction");
                else if (target != pluginId && !(od == "keep" || (od == "set_null" && target == "platform")))
                    error("E_REF_CROSS_ON_DELETE", dp, target == "platform"
                        ? "a ref to the directory keeps the id (Keep) or is cleared (SetNull)"
                        : "a ref to another plugin is not a foreign key: it keeps the id (Keep)");
                else if (od == "set_null" && !f.Optional)
                    error("E_REF_ON_DELETE", dp, $"{f.Name} is required: SetNull clears an optional ref");
                else if (od == "set_null" && appendOnly)
                    error("E_ENT_APPEND_ONLY_CASCADE", dp, "an append-only entity's rows never change: no SetNull");
            }
        }
    }

    private static JsonArray Strings(IEnumerable<string> values) => new([.. values.Select(v => (JsonNode)v)]);

    /// <summary>
    /// Copies each <c>upload</c> input's file field options into it (<c>multiple</c> for a list of files,
    /// <c>max_size</c>, <c>accept</c>), from the request of its form's endpoint, inputs of a <c>repeat</c> included.
    /// </summary>
    private static void FillUploads(JsonNode? node, JsonArray endpoints, JsonArray messages)
    {
        switch (node)
        {
            case JsonObject o:
                if (o["form"] is JsonObject form
                    && form["endpoint"]?.GetValueKind() == JsonValueKind.String
                    && endpoints.FirstOrDefault(e => (string?)e?["name"] == (string)form["endpoint"]!) is { } endpoint
                    && form["sections"] is JsonArray sections)
                    foreach (var s in sections)
                        if (s?["inputs"] is JsonArray inputs)
                            FillUploadInputs(inputs, (string)endpoint["input"]!, messages);
                foreach (var (_, v) in o) FillUploads(v, endpoints, messages);
                break;
            case JsonArray a:
                foreach (var v in a) FillUploads(v, endpoints, messages);
                break;
        }
    }

    private static void FillUploadInputs(JsonArray inputs, string message, JsonArray messages)
    {
        foreach (var node in inputs)
        {
            if (node is not JsonObject input || input["name"]?.GetValueKind() != JsonValueKind.String) continue;
            if (FieldAt(messages, message, (string)input["name"]!) is not { } f) continue;
            var type = (string?)f["type"] ?? "";
            if ((string?)input["control"] == "upload" && type == "file")
            {
                if ((bool?)f["repeated"] == true) input["multiple"] = true;
                if (f["max_size"] is { } max) input["max_size"] = max.DeepClone();
                if (f["accept"] is JsonArray accept && accept.Count > 0) input["accept"] = accept.DeepClone();
            }
            if (type.StartsWith("message:", StringComparison.Ordinal) && input["fields"] is JsonArray nested)
                FillUploadInputs(nested, type["message:".Length..], messages);
        }
    }

    /// <summary>A field of a message by its path (<c>customer.name</c>), through nested messages.</summary>
    private static JsonObject? FieldAt(JsonArray messages, string message, string path)
    {
        var dot = path.IndexOf('.');
        var (head, rest) = dot < 0 ? (path, null) : (path[..dot], path[(dot + 1)..]);
        var m = messages.FirstOrDefault(x => (string?)x?["name"] == message);
        if (m?["fields"] is not JsonArray fields || fields.FirstOrDefault(x => (string?)x?["name"] == head) is not JsonObject f)
            return null;
        if (rest is null) return f;
        var type = (string?)f["type"] ?? "";
        return type.StartsWith("message:", StringComparison.Ordinal) ? FieldAt(messages, type["message:".Length..], rest) : null;
    }

    /// <summary>Messages in order of first use (a message, then the messages its fields use); enums in order of first use.</summary>
    private sealed class Walk
    {
        public List<Type> Messages { get; } = [];
        public List<Type> Enums { get; } = [];
        private readonly HashSet<Type> _seen = [];

        public void Message(Type t)
        {
            if (!_seen.Add(t)) return;
            Messages.Add(t);
            foreach (var f in MessageSchema.For(t).Fields)
                if (f.Kind == FieldKind.Message) Message(f.ElementType);
                else if (f.Kind == FieldKind.Enum) Enum(f.ElementType);
        }

        public void Enum(Type t)
        {
            if (_seen.Add(t)) Enums.Add(t);
        }
    }

    [GeneratedRegex("^[a-z][a-z0-9]{1,15}$")]
    private static partial Regex PluginId();

    [GeneratedRegex("^[a-z][a-z0-9_]{1,31}$")]
    private static partial Regex RoleName();

    [GeneratedRegex("^[a-z][a-z0-9_]{0,31}$")]
    private static partial Regex LakeName();

    [GeneratedRegex("^[a-z][a-z0-9_.]{0,62}$")]
    private static partial Regex ScopeName();

    [GeneratedRegex("^[a-z][a-z0-9_]{0,62}$")]
    private static partial Regex SqlName();

    [GeneratedRegex("^[a-z][a-z0-9_]{0,31}$")]
    private static partial Regex KindName();

    [GeneratedRegex("^[A-Za-z0-9_.:-]+$")]
    private static partial Regex JobPathName();

    // semver 2.0.0
    [GeneratedRegex(@"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$")]
    private static partial Regex SemVer();
}
