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
            entities.Add(entity);
        }
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
                var connection = new JsonObject { ["name"] = c.Name, ["kind"] = c.Kind == ConnectionKind.Llm ? "llm" : "stt" };
                if (c.TitleText.Length > 0) connection["title"] = c.TitleText;
                if (c.Description.Length > 0) connection["description"] = c.Description;
                return (JsonNode)connection;
            })]);
        if (p.Topics.Count > 0) json["topics"] = new JsonArray([.. p.Topics.Select(t => (JsonNode)t.ToJson())]);
        if (p.Ui is not null) json["ui"] = p.Ui.Build();

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
        // the background jobs: written only when declared
        var jobs = Jobs(p, Error);
        if (jobs.Count > 0) json["jobs"] = jobs;
        return (json, errors);
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
