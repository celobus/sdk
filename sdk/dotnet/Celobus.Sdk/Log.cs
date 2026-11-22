// Logs: one JSON object per line on stderr (stdout is the descriptor's under --describe). Level from
// $CELOBUS_LOG (trace, debug, info, warn, error; default info). A job's run also keeps its lines (RunLog.cs).
using System.Globalization;
using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using Microsoft.Extensions.Logging;

namespace Celobus.Sdk;

internal sealed class JsonLogger(string category, IReadOnlyList<KeyValuePair<string, object?>> context) : ILogger
{
    private static readonly JsonWriterOptions WriterOptions = new() { Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping };
    private static readonly object Gate = new();

    internal static TextWriter Output { get; set; } = Console.Error;
    internal static LogLevel MinLevel { get; set; } = Parse(Environment.GetEnvironmentVariable("CELOBUS_LOG"));

    public JsonLogger With(params KeyValuePair<string, object?>[] more) => new(category, [.. context, .. more]);

    public IDisposable? BeginScope<TState>(TState state) where TState : notnull => null;

    public bool IsEnabled(LogLevel logLevel) => logLevel != LogLevel.None && logLevel >= MinLevel;

    public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception,
        Func<TState, Exception?, string> formatter)
    {
        if (!IsEnabled(logLevel)) return;
        var msg = formatter(state, exception);
        RunLog.Write(logLevel, state, exception, msg);
        using var buffer = new MemoryStream();
        using (var w = new Utf8JsonWriter(buffer, WriterOptions))
        {
            w.WriteStartObject();
            w.WriteString("ts", TimestampConverter.Format(DateTimeOffset.UtcNow));
            w.WriteString("level", Level(logLevel));
            w.WriteString("target", category);
            w.WriteString("msg", msg);
            foreach (var (k, v) in context) Write(w, k, v);
            if (state is IEnumerable<KeyValuePair<string, object?>> fields)
                foreach (var (k, v) in fields)
                    if (k != "{OriginalFormat}") Write(w, k, v);
            if (exception != null) w.WriteString("error", exception.ToString());
            w.WriteEndObject();
        }
        var line = Encoding.UTF8.GetString(buffer.GetBuffer(), 0, (int)buffer.Length);
        lock (Gate) Output.WriteLine(line);
    }

    private static void Write(Utf8JsonWriter w, string key, object? value)
    {
        switch (value)
        {
            case null: w.WriteNull(key); break;
            case bool b: w.WriteBoolean(key, b); break;
            case int or long or uint or ulong or short or ushort or byte:
                w.WriteNumber(key, Convert.ToDecimal(value, CultureInfo.InvariantCulture)); break;
            case double d when double.IsFinite(d): w.WriteNumber(key, d); break;
            case TimeSpan t: w.WriteNumber(key + "_ms", Math.Round(t.TotalMilliseconds, 3)); break;
            default: w.WriteString(key, Convert.ToString(value, CultureInfo.InvariantCulture)); break;
        }
    }

    private static string Level(LogLevel l) => l switch
    {
        LogLevel.Trace => "trace",
        LogLevel.Debug => "debug",
        LogLevel.Information => "info",
        LogLevel.Warning => "warn",
        LogLevel.Error => "error",
        _ => "fatal",
    };

    private static LogLevel Parse(string? s) => s?.Trim().ToLowerInvariant() switch
    {
        "trace" => LogLevel.Trace,
        "debug" => LogLevel.Debug,
        "warn" or "warning" => LogLevel.Warning,
        "error" => LogLevel.Error,
        "off" or "none" => LogLevel.None,
        _ => LogLevel.Information,
    };
}
