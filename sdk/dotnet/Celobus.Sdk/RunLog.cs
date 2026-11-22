// The run log: what a job's handler logs through the SDK's loggers (ctx.Log) while it runs is kept with the run, by
// attempt, and shown with it; stderr gets the line as always. Lines are the message, then the fields the message does
// not already name as `key=value`, cut at 2 KiB; a process sends at most 1,000 lines and 1 MiB a second and drops
// the rest. Lines logged while handling a request are not kept this way.
using System.Globalization;
using System.Text;
using Celobus.V1;
using Microsoft.Extensions.Logging;

namespace Celobus.Sdk;

/// <summary>Where a run's lines go: the run's request id and the session.</summary>
internal sealed record RunLogTarget(ulong RequestId, Func<PluginMessage, bool> Send);

internal static class RunLog
{
    /// <summary>The longest line kept, in UTF-8 bytes.</summary>
    public const int MaxLine = 2048;

    private static readonly AsyncLocal<RunLogTarget?> CurrentRun = new();
    private static readonly Limiter Limit = new();

    /// <summary>The run the code calling is part of (null: none, a request's code).</summary>
    public static RunLogTarget? Current
    {
        get => CurrentRun.Value;
        set => CurrentRun.Value = value;
    }

    /// <summary>Sends a logged line with the current run, if there is one and the limit lets it go.</summary>
    public static void Write<TState>(LogLevel level, TState state, Exception? exception, string message)
    {
        var run = CurrentRun.Value;
        if (run is null) return;
        var line = Cut(Line(state, exception, message), MaxLine);
        if (!Limit.Admit(Environment.TickCount64, Encoding.UTF8.GetByteCount(line))) return;
        run.Send(Message(run.RequestId, level, line, DateTimeOffset.UtcNow));
    }

    internal static PluginMessage Message(ulong requestId, LogLevel level, string line, DateTimeOffset at) => new()
    {
        PluginLog = new PluginLog
        {
            RequestId = requestId,
            Level = level switch
            {
                LogLevel.Error or LogLevel.Critical => PluginLog.Types.Level.Error,
                LogLevel.Warning => PluginLog.Types.Level.Warn,
                LogLevel.Information => PluginLog.Types.Level.Info,
                _ => PluginLog.Types.Level.Debug,
            },
            Message = line,
            AtUnixMs = at.ToUnixTimeMilliseconds(),
        },
    };

    /// <summary>The message, then the state's fields that the message template does not name, as <c>key=value</c>.</summary>
    internal static string Line<TState>(TState state, Exception? exception, string message)
    {
        var sb = new StringBuilder(message);
        if (state is IEnumerable<KeyValuePair<string, object?>> fields)
        {
            var template = "";
            foreach (var (k, v) in fields)
                if (k == "{OriginalFormat}") template = v as string ?? "";
            foreach (var (k, v) in fields)
            {
                if (k == "{OriginalFormat}" || Named(template, k)) continue;
                if (sb.Length > 0) sb.Append(' ');
                sb.Append(k).Append('=').Append(Value(v));
            }
        }
        if (exception is not null)
        {
            if (sb.Length > 0) sb.Append(' ');
            sb.Append("error=").Append(Value($"{exception.GetType().Name}: {exception.Message}"));
        }
        return sb.ToString();
    }

    // `{name}`, `{name:format}`, `{name,width}`, `{@name}`, `{$name}`
    private static bool Named(string template, string name) =>
        template.Contains("{" + name + "}", StringComparison.OrdinalIgnoreCase)
        || template.Contains("{" + name + ":", StringComparison.OrdinalIgnoreCase)
        || template.Contains("{" + name + ",", StringComparison.OrdinalIgnoreCase)
        || template.Contains("{@" + name, StringComparison.OrdinalIgnoreCase)
        || template.Contains("{$" + name, StringComparison.OrdinalIgnoreCase);

    private static string Value(object? v) => v switch
    {
        null => "null",
        string s => "\"" + s.Replace("\\", "\\\\").Replace("\"", "\\\"") + "\"",
        bool b => b ? "true" : "false",
        IFormattable f => f.ToString(null, CultureInfo.InvariantCulture),
        _ => v.ToString() ?? "",
    };

    /// <summary>At most <paramref name="maxBytes"/> UTF-8 bytes of <paramref name="s"/>, cut at a character boundary.</summary>
    internal static string Cut(string s, int maxBytes)
    {
        if (s.Length * 3 <= maxBytes || Encoding.UTF8.GetByteCount(s) <= maxBytes) return s;
        var bytes = 0;
        var i = 0;
        while (i < s.Length)
        {
            var pair = char.IsHighSurrogate(s[i]) && i + 1 < s.Length && char.IsLowSurrogate(s[i + 1]);
            var n = pair ? 4 : s[i] < 0x80 ? 1 : s[i] < 0x800 ? 2 : 3;
            if (bytes + n > maxBytes) break;
            bytes += n;
            i += pair ? 2 : 1;
        }
        return s[..i];
    }
}

/// <summary>A fixed one-second window of lines and bytes: at most 1,000 lines and 1 MiB a second.</summary>
internal sealed class Limiter
{
    public const int LinesPerSecond = 1_000;
    public const int BytesPerSecond = 1 << 20;

    private readonly Lock _gate = new();
    private long? _window;
    private int _lines;
    private long _bytes;

    /// <summary>Whether a line of <paramref name="bytes"/> may go at <paramref name="nowMs"/> (a monotonic clock).</summary>
    public bool Admit(long nowMs, int bytes)
    {
        lock (_gate)
        {
            if (_window is not { } start || nowMs - start >= 1000)
            {
                _window = nowMs;
                _lines = 0;
                _bytes = 0;
            }
            if (_lines >= LinesPerSecond || _bytes + bytes > BytesPerSecond) return false;
            _lines++;
            _bytes += bytes;
            return true;
        }
    }
}
