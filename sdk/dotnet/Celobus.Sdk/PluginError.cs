using System.Text.Json.Nodes;
using System.Text.RegularExpressions;

namespace Celobus.Sdk;

/// <summary>
/// An error a handler answers with: <c>throw PluginError.NotFound("no such note")</c>. It becomes the HTTP status
/// and <c>{"code", "message", "details"}</c>. Any other exception a handler throws is a 500 <c>internal</c> error
/// whose message is hidden (and logged).
/// <para>
/// In a job's handler an error fails the attempt, and the run is retried unless the error is permanent
/// (<see cref="IsPermanent"/>): <see cref="Permanent"/>, an error marked with <see cref="AsPermanent"/>, or an
/// input-class error (<c>invalid</c>, <c>not_found</c>, <c>permission_denied</c>, <c>payload_invalid</c>).
/// </para>
/// </summary>
public sealed partial class PluginError : Exception
{
    /// <summary>The codes of input-class errors: a job's attempt failing with one of them is not retried.</summary>
    private static readonly HashSet<string> PermanentCodes = ["invalid", "not_found", "permission_denied", "payload_invalid"];

    private readonly Dictionary<string, string> _fields = new(StringComparer.Ordinal);
    private bool _permanent;

    /// <summary>An error with its own status and code (a dependency's error is forwarded this way).</summary>
    public PluginError(int status, string code, string message, JsonObject? details = null, Exception? inner = null)
        : base(message, inner)
    {
        Status = status;
        Code = code;
        Details = details;
    }

    /// <summary>The HTTP status.</summary>
    public int Status { get; }

    /// <summary>The error code (<c>invalid</c>, <c>not_found</c>, a domain code …).</summary>
    public string Code { get; }

    /// <summary>Details beyond the invalid fields (a forwarded dependency error keeps its own).</summary>
    public JsonObject? Details { get; }

    /// <summary>The invalid fields of an <c>invalid</c> error: path → message.</summary>
    public IReadOnlyDictionary<string, string> Fields => _fields;

    /// <summary>400 <c>invalid</c>; add the fields at fault with <see cref="Field"/>.</summary>
    public static PluginError Invalid(string message) => new(400, "invalid", message);

    /// <summary>401 <c>unauthenticated</c>: the request has no user (or the handler needs one).</summary>
    public static PluginError Unauthenticated(string message = "a signed-in user is required") =>
        new(401, "unauthenticated", message);

    /// <summary>403 <c>forbidden</c>.</summary>
    public static PluginError Forbidden(string message = "forbidden") => new(403, "forbidden", message);

    /// <summary>404 <c>not_found</c>.</summary>
    public static PluginError NotFound(string message = "not found") => new(404, "not_found", message);

    /// <summary>409 <c>conflict</c>.</summary>
    public static PluginError Conflict(string message) => new(409, "conflict", message);

    /// <summary>503 <c>unavailable</c>: a transient failure, safe to retry.</summary>
    public static PluginError Unavailable(string message) => new(503, "unavailable", message);

    /// <summary>503 <c>unavailable</c> wrapping an outside failure (its message is kept).</summary>
    public static PluginError Unavailable(Exception inner) => new(503, "unavailable", inner.Message, null, inner);

    /// <summary>
    /// 422 <c>permanent</c>: in a job, the attempt fails and the run is not retried (bad input that a retry cannot fix).
    /// </summary>
    public static PluginError Permanent(string message) => new PluginError(422, "permanent", message).AsPermanent();

    /// <summary>Marks this error permanent: a job's run that fails with it is not retried; returns this error.</summary>
    public PluginError AsPermanent()
    {
        _permanent = true;
        return this;
    }

    /// <summary>
    /// Whether a job's run that fails with this error is not retried: marked permanent, or an input-class error
    /// (<c>invalid</c>, <c>not_found</c>, <c>permission_denied</c>, <c>payload_invalid</c>).
    /// </summary>
    public bool IsPermanent => _permanent || PermanentCodes.Contains(Code);

    /// <summary>A domain error: <c>PluginError.Domain("not_cancellable", 409, "the order is shipped")</c>; status 400–422.</summary>
    public static PluginError Domain(string code, int status, string message)
    {
        if (!CodePattern().IsMatch(code)) throw new ArgumentException($"`{code}` is not a snake_case error code", nameof(code));
        if (status is < 400 or > 422) throw new ArgumentOutOfRangeException(nameof(status), status, "a domain error's status is 400–422");
        return new PluginError(status, code, message);
    }

    /// <summary>Marks a field of the request as invalid (its snake_case path, <c>lines[2].quantity</c>); returns this error.</summary>
    public PluginError Field(string path, string message)
    {
        _fields[path] = message;
        return this;
    }

    /// <summary>The 500 <c>internal</c> answer; its message is never shown.</summary>
    internal static PluginError Internal() => new(500, "internal", "internal error");

    /// <summary>The body: <c>{"code", "message", "details"}</c>.</summary>
    internal JsonObject ToJson()
    {
        var details = Details?.DeepClone().AsObject() ?? new JsonObject();
        if (_fields.Count > 0)
        {
            var fields = new JsonObject();
            foreach (var (k, v) in _fields) fields[k] = v;
            details["fields"] = fields;
        }
        return new JsonObject { ["code"] = Code, ["message"] = Message, ["details"] = details };
    }

    /// <summary>Reads an error body of another plugin (or of the platform); null when it is not one.</summary>
    internal static PluginError? FromBody(int status, ReadOnlySpan<byte> body)
    {
        try
        {
            if (JsonNode.Parse(body) is not JsonObject o || o["code"]?.GetValue<string>() is not { } code) return null;
            var message = o["message"]?.GetValue<string>() ?? "";
            var details = o["details"] as JsonObject;
            var error = new PluginError(status, code, message, details?.DeepClone().AsObject());
            return error;
        }
        catch (Exception)
        {
            return null;
        }
    }

    [GeneratedRegex("^[a-z][a-z0-9_]*$")]
    private static partial Regex CodePattern();
}
