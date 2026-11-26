// The plugin's files (`ctx.Files`): reading a file's bytes as a Stream, writing a new file, what is known of one. The
// bytes never pass the plugin session: the platform answers signed URLs of the workspace's object store and the SDK
// reads (ranged GETs, resumed past a URL's expiry or a dropped connection) and writes (parts of a multipart upload,
// one part buffered) them there. The in-memory store of `Ctx.Test` answers the same calls (MemoryFiles).
using System.Globalization;
using System.Net;
using System.Net.Http.Headers;

namespace Celobus.Sdk;

/// <summary>What is known of a file (<see cref="Files.Head"/>).</summary>
/// <param name="File">The file.</param>
/// <param name="Etag">Its bytes' entity tag in the store (the same bytes, the same tag).</param>
/// <param name="CreatedAt">When it was uploaded or made.</param>
/// <param name="CreatedBy">Who uploaded or made it; null when the plugin itself made it outside a request.</param>
public sealed record FileDetails(File File, string Etag, DateTimeOffset CreatedAt, Id<Platform.User>? CreatedBy);

/// <summary>
/// The plugin's files (<c>ctx.Files</c>): <see cref="Open"/> reads one as a <see cref="Stream"/>,
/// <see cref="Create"/> writes a new one, <see cref="Head"/> tells what is known of one. A plugin reaches only its own
/// files: those it was given (a request's or a job payload's <c>File</c> fields, a call's answer, a row it reads) and
/// those it made; any other id is 404 <c>not_found</c>, a file collected since 410 <c>file_gone</c>. The platform is
/// unreachable: 503 <c>unavailable</c>. There is no delete: a file goes when nothing holds it (remove it from the
/// record).
/// </summary>
public readonly struct Files
{
    private readonly Ctx _ctx;

    internal Files(Ctx ctx) { _ctx = ctx; }

    /// <summary>
    /// Opens <paramref name="file"/> for reading, from byte <paramref name="offset"/>: a read-only
    /// <see cref="Stream"/> (<c>new StreamReader(await ctx.Files.Open(job.File))</c>) whose <c>Length</c> is the file's
    /// size; it seeks (each seek reads on from there). The bytes are streamed, never held whole; a read resumes where it
    /// stopped when its connection drops or its URL expires. Dispose it when done.
    /// </summary>
    public async Task<Stream> Open(File file, long offset = 0)
    {
        ArgumentNullException.ThrowIfNull(file);
        ArgumentOutOfRangeException.ThrowIfNegative(offset);
        var read = await OpenAt(_ctx, file.Id, offset);
        var stream = new FileReadStream(_ctx, file.Id, read, offset);
        try
        {
            await stream.Connect(_ctx.Cancelled);
        }
        catch
        {
            await stream.DisposeAsync();
            throw;
        }
        return stream;
    }

    /// <summary>
    /// Starts a new file of the plugin named <paramref name="name"/> (at most 255 bytes; <c>/</c>, <c>\</c>, control
    /// characters and leading dots are removed; empty is 400 <c>bad_request</c>) of type
    /// <paramref name="contentType"/> (empty: <c>application/octet-stream</c>): a <see cref="FileWriter"/> to write its
    /// bytes to and <see cref="FileWriter.Finish"/>. With <paramref name="size"/> the storage is reserved at once and
    /// writing more or less is 409 <c>file_size_mismatch</c>; without, part by part. Inside a request the file is
    /// made for its caller, who may download it while no record holds it (an export answered as a <c>File</c>); a file
    /// nothing holds is collected after 24 hours. 409 <c>read_only</c> in a read-only endpoint, 413
    /// <c>file_too_large</c>, 507 <c>quota_exceeded</c> when the workspace's storage is full (also from a later write).
    /// </summary>
    public async Task<FileWriter> Create(string name, string contentType, long? size = null)
    {
        if (size is < 0) throw new ArgumentOutOfRangeException(nameof(size), "a size is 0 bytes or more");
        var call = new V1.FilesCreate { Name = name ?? "", ContentType = contentType ?? "", RequestId = _ctx.Request.Id };
        if (size is { } n) call.Size = (ulong)n;
        var upload = (await _ctx.Syscall(new V1.Syscall { FilesCreate = call })).FilesUpload ?? throw Unexpected();
        return new FileWriter(_ctx, upload, size);
    }

    /// <summary>What is known of <paramref name="file"/>: its name, type and size, its entity tag, who made it and when.</summary>
    public async Task<FileDetails> Head(File file)
    {
        ArgumentNullException.ThrowIfNull(file);
        var call = new V1.FilesHead { Id = file.Id, RequestId = _ctx.Request.Id };
        var info = (await _ctx.Syscall(new V1.Syscall { FilesHead = call })).FileInfo ?? throw Unexpected();
        return new FileDetails(From(info.File ?? throw Unexpected()), info.Etag,
            DateTimeOffset.FromUnixTimeMilliseconds(info.CreatedAtMs),
            info.CreatedBy.Length == 0 ? null : new Id<Platform.User>(info.CreatedBy));
    }

    internal static async Task<V1.FilesRead> OpenAt(Ctx ctx, string id, long offset)
    {
        var call = new V1.FilesOpen { Id = id, RequestId = ctx.Request.Id };
        if (offset > 0) call.Offset = (ulong)offset;
        var read = (await ctx.Syscall(new V1.Syscall { FilesOpen = call })).FilesRead ?? throw Unexpected();
        return read.File is null ? throw Unexpected() : read;
    }

    internal static File From(V1.File f) => new(f.Id, f.Name, f.ContentType, (long)Math.Min(f.Size, long.MaxValue));

    internal static V1.File Wire(File f) => new()
    {
        Id = f.Id,
        Name = f.Name ?? "",
        ContentType = f.ContentType ?? "",
        Size = (ulong)Math.Max(f.Size, 0),
    };

    internal static PluginError Unexpected() => PluginError.Unavailable("platform: unexpected syscall result");
}

/// <summary>How the SDK reaches the store's signed URLs.</summary>
internal static class FileTransfer
{
    /// <summary>One client for the process: its connections are pooled; a transfer's own token ends it.</summary>
    public static readonly HttpClient Client = new(new SocketsHttpHandler
    {
        PooledConnectionLifetime = TimeSpan.FromMinutes(5),
        AutomaticDecompression = DecompressionMethods.None,
    })
    {
        Timeout = Timeout.InfiniteTimeSpan,
    };

    /// <summary>Tries of one request (a part, a connection) before the transfer fails.</summary>
    public const int Attempts = 4;

    /// <summary>A URL this close to its expiry is renewed before use.</summary>
    public static readonly TimeSpan Margin = TimeSpan.FromSeconds(30);

    public static TimeSpan Backoff(int attempt) => TimeSpan.FromMilliseconds(100 * (1 << Math.Min(attempt, 6)));

    public static DateTimeOffset At(long ms) => ms <= 0 ? DateTimeOffset.MaxValue : DateTimeOffset.FromUnixTimeMilliseconds(ms);

    public static PluginError Failed(string id, string what) => PluginError.Unavailable($"file {id}: {what}");
}

/// <summary>
/// A file being read (<see cref="Files.Open"/>): ranged GETs of a signed URL, renewed when it expires and resumed at
/// the position when a connection drops.
/// </summary>
internal sealed class FileReadStream : Stream
{
    private readonly Ctx _ctx;
    private readonly string _id;
    private readonly long _length;
    private string _url;
    private DateTimeOffset _expires;
    private HttpResponseMessage? _response;
    private Stream? _body;
    private long _position;

    internal FileReadStream(Ctx ctx, string id, V1.FilesRead read, long offset)
    {
        _ctx = ctx;
        _id = id;
        _length = (long)Math.Min(read.File.Size, long.MaxValue);
        _url = read.Url;
        _expires = FileTransfer.At(read.ExpiresAtMs);
        _position = offset;
    }

    public override bool CanRead => true;
    public override bool CanSeek => true;
    public override bool CanWrite => false;
    public override long Length => _length;

    public override long Position
    {
        get => _position;
        set => Seek(value, SeekOrigin.Begin);
    }

    public override long Seek(long offset, SeekOrigin origin)
    {
        var to = origin switch
        {
            SeekOrigin.Begin => offset,
            SeekOrigin.Current => _position + offset,
            _ => _length + offset,
        };
        ArgumentOutOfRangeException.ThrowIfNegative(to, nameof(offset));
        if (to != _position)
        {
            Drop();
            _position = to;
        }
        return _position;
    }

    /// <summary>A GET from the position (a <c>Range</c> past 0), renewing the URL when it is about to expire or has.</summary>
    internal async Task Connect(CancellationToken ct)
    {
        if (_length > 0 && _position >= _length)
        {
            // at or past the end: nothing more to read
            _body = Stream.Null;
            return;
        }
        for (var attempt = 1; ; attempt++)
        {
            if (DateTimeOffset.UtcNow + FileTransfer.Margin >= _expires) await Renew();
            using var request = new HttpRequestMessage(HttpMethod.Get, _url);
            if (_position > 0) request.Headers.Range = new RangeHeaderValue(_position, null);
            HttpResponseMessage response;
            try
            {
                response = await _ctx.Http.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, ct);
            }
            catch (HttpRequestException e)
            {
                if (attempt >= FileTransfer.Attempts) throw FileTransfer.Failed(_id, $"the store cannot be reached ({e.Message})");
                await Task.Delay(FileTransfer.Backoff(attempt), ct);
                continue;
            }
            var status = (int)response.StatusCode;
            if (status is 200 or 206)
            {
                _response = response;
                _body = await response.Content.ReadAsStreamAsync(ct);
                // the store sent the whole file: read up to the position
                if (status == 200 && _position > 0) await Skip(_body, _position, ct);
                return;
            }
            response.Dispose();
            if (status == 416)
            {
                // at or past the end: nothing more to read
                _body = Stream.Null;
                return;
            }
            if (status == 404) throw new PluginError(410, "file_gone", $"the file {_id} was collected");
            if (attempt >= FileTransfer.Attempts) throw FileTransfer.Failed(_id, $"the store answered {status}");
            // 403: the URL expired (or was signed for another time); 5xx: the store is busy
            if (status is 401 or 403) _expires = DateTimeOffset.MinValue;
            else if (status >= 500 || status == 429) await Task.Delay(FileTransfer.Backoff(attempt), ct);
            else throw FileTransfer.Failed(_id, $"the store answered {status}");
        }
    }

    private async Task Renew()
    {
        var read = await Files.OpenAt(_ctx, _id, _position);
        _url = read.Url;
        _expires = FileTransfer.At(read.ExpiresAtMs);
    }

    private static async Task Skip(Stream body, long count, CancellationToken ct)
    {
        var scratch = new byte[(int)Math.Min(count, 81920)];
        while (count > 0)
        {
            var n = await body.ReadAsync(scratch.AsMemory(0, (int)Math.Min(count, scratch.Length)), ct);
            if (n == 0) throw new IOException("the connection ended early");
            count -= n;
        }
    }

    public override async ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken ct = default)
    {
        if (buffer.Length == 0 || _position >= _length) return 0;
        if (!ct.CanBeCanceled) ct = _ctx.Cancelled;
        for (var failures = 1; ; failures++)
        {
            try
            {
                if (_body is null) await Connect(ct);
                var n = await _body!.ReadAsync(buffer, ct);
                if (n == 0)
                {
                    if (_position >= _length || _body == Stream.Null) return 0;
                    throw new IOException("the connection ended early");
                }
                _position += n;
                return n;
            }
            catch (Exception e) when (e is IOException or HttpRequestException && !ct.IsCancellationRequested)
            {
                // read on from the position over a new connection
                Drop();
                if (failures >= FileTransfer.Attempts) throw FileTransfer.Failed(_id, $"reading at byte {_position}: {e.Message}");
                await Task.Delay(FileTransfer.Backoff(failures), ct);
            }
        }
    }

    public override Task<int> ReadAsync(byte[] buffer, int offset, int count, CancellationToken ct) =>
        ReadAsync(buffer.AsMemory(offset, count), ct).AsTask();

    public override int Read(byte[] buffer, int offset, int count) =>
        ReadAsync(buffer.AsMemory(offset, count)).AsTask().GetAwaiter().GetResult();

    public override void Flush() { }
    public override void SetLength(long value) => throw new NotSupportedException("a file is read-only");
    public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException("a file is read-only");

    private void Drop()
    {
        if (_body != Stream.Null) _body?.Dispose();
        _body = null;
        _response?.Dispose();
        _response = null;
    }

    protected override void Dispose(bool disposing)
    {
        if (disposing) Drop();
        base.Dispose(disposing);
    }
}

/// <summary>
/// A new file being written (<see cref="Files.Create"/>): a write-only <see cref="Stream"/>
/// (<c>await w.WriteAsync(bytes)</c>, <c>CopyToAsync(w)</c>, a <c>StreamWriter</c> over it), then
/// <see cref="Finish"/> for the <see cref="File"/>. It holds one part of the file in memory (16 MiB) and sends each
/// part as it fills; the bytes are not part of the file until <see cref="Finish"/>. A <c>StreamWriter</c> over it
/// writes text: flush or dispose it, then <see cref="Finish"/>. Disposing the writer without finishing abandons the
/// file (its storage is freed within the hour). Not for use by several tasks at once.
/// </summary>
public sealed class FileWriter : Stream
{
    private readonly Ctx _ctx;
    private readonly string _id;
    private readonly int _partSize;
    private readonly long? _size;
    private readonly Dictionary<int, string> _urls = [];
    private readonly List<string?> _etags = [];
    private DateTimeOffset _expires;
    private byte[]? _buffer;
    private int _buffered;
    private int _next = 1;
    private long _written;
    private bool _finished;
    private bool _broken;

    internal FileWriter(Ctx ctx, V1.FilesUpload upload, long? size)
    {
        _ctx = ctx;
        _id = upload.Id;
        _partSize = (int)Math.Clamp(upload.PartSize, 1UL, (ulong)Array.MaxLength);
        _size = size;
        Page(upload);
    }

    /// <summary>The id of the file being written.</summary>
    public string Id => _id;

    /// <summary>The bytes written so far.</summary>
    public long Written => _written;

    public override bool CanRead => false;
    public override bool CanSeek => false;
    public override bool CanWrite => !_finished;
    public override long Length => _written;

    public override long Position
    {
        get => _written;
        set => throw new NotSupportedException("a file is written from start to end");
    }

    private void Page(V1.FilesUpload page)
    {
        _expires = FileTransfer.At(page.ExpiresAtMs);
        for (var i = 0; i < page.Urls.Count; i++) _urls[(int)page.First + i] = page.Urls[i];
    }

    private async Task<string> Url(int part)
    {
        if (_urls.TryGetValue(part, out var url) && DateTimeOffset.UtcNow + FileTransfer.Margin < _expires) return url;
        // the next page of URLs (or the same ones signed again), from this part on
        var call = new V1.FilesParts { Id = _id, From = (uint)part, RequestId = _ctx.Request.Id };
        _urls.Clear();
        Page((await _ctx.Syscall(new V1.Syscall { FilesParts = call })).FilesUpload ?? throw Files.Unexpected());
        return _urls.TryGetValue(part, out url) ? url : throw FileTransfer.Failed(_id, $"no URL for part {part}");
    }

    private void Writable(int count)
    {
        if (_finished) throw new InvalidOperationException("the file is finished: make a new one to write more");
        if (_broken) throw FileTransfer.Failed(_id, "a part could not be written: make the file again");
        if (_size is { } n && _written + count > n)
            throw new PluginError(409, "file_size_mismatch", $"file {_id}: more than the {n} bytes declared");
    }

    public override void Write(ReadOnlySpan<byte> data)
    {
        Writable(data.Length);
        while (data.Length > 0)
        {
            var n = Take(data);
            data = data[n..];
            if (_buffered == _partSize) PutPart(_ctx.Cancelled).GetAwaiter().GetResult();
        }
    }

    public override void Write(byte[] buffer, int offset, int count) => Write(buffer.AsSpan(offset, count));

    public override async ValueTask WriteAsync(ReadOnlyMemory<byte> data, CancellationToken ct = default)
    {
        Writable(data.Length);
        while (data.Length > 0)
        {
            var n = Take(data.Span);
            data = data[n..];
            if (_buffered == _partSize) await PutPart(ct.CanBeCanceled ? ct : _ctx.Cancelled);
        }
    }

    public override Task WriteAsync(byte[] buffer, int offset, int count, CancellationToken ct) =>
        WriteAsync(buffer.AsMemory(offset, count), ct).AsTask();

    /// <summary>Copies into the part being filled; how many bytes it took.</summary>
    private int Take(ReadOnlySpan<byte> data)
    {
        _buffer ??= new byte[_size is { } n ? (int)Math.Clamp(n, 1, _partSize) : _partSize];
        var take = Math.Min(data.Length, _partSize - _buffered);
        if (_buffered + take > _buffer.Length) Array.Resize(ref _buffer, _partSize);
        data[..take].CopyTo(_buffer.AsSpan(_buffered));
        _buffered += take;
        _written += take;
        return take;
    }

    /// <summary>Sends the part being filled (exactly its length), retrying a dropped connection or an expired URL.</summary>
    private async Task PutPart(CancellationToken ct)
    {
        var part = _next;
        for (var attempt = 1; ; attempt++)
        {
            var url = await Url(part);
            using var request = new HttpRequestMessage(HttpMethod.Put, url)
            {
                Content = new ByteArrayContent(_buffer ?? [], 0, _buffered),
            };
            HttpResponseMessage response;
            try
            {
                response = await _ctx.Http.SendAsync(request, ct);
            }
            catch (HttpRequestException e)
            {
                if (attempt >= FileTransfer.Attempts) throw Broken($"the store cannot be reached ({e.Message})");
                await Task.Delay(FileTransfer.Backoff(attempt), ct);
                continue;
            }
            using (response)
            {
                var status = (int)response.StatusCode;
                if (response.IsSuccessStatusCode)
                {
                    // the tag as the store wrote it, quotes and all
                    _etags.Add(response.Headers.TryGetValues("ETag", out var tags) ? tags.FirstOrDefault() : null);
                    _next++;
                    _buffered = 0;
                    return;
                }
                if (attempt >= FileTransfer.Attempts) throw Broken($"the store answered {status} to part {part}");
                if (status is 401 or 403) _expires = DateTimeOffset.MinValue; // the URL expired: sign the page again
                else if (status >= 500 || status == 429) await Task.Delay(FileTransfer.Backoff(attempt), ct);
                else throw Broken($"the store answered {status} to part {part}");
            }
        }
    }

    private PluginError Broken(string why)
    {
        _broken = true;
        return FileTransfer.Failed(_id, why);
    }

    /// <summary>
    /// Sends what is left and ends the file: the <see cref="File"/>, ready to keep in a row, a job payload or an
    /// answer. 409 <c>file_size_mismatch</c> when a declared size was not written; 507 <c>quota_exceeded</c>.
    /// </summary>
    public async Task<File> Finish()
    {
        if (_finished) throw new InvalidOperationException("the file is finished already");
        Writable(0);
        if (_size is { } n && _written != n)
            throw new PluginError(409, "file_size_mismatch", $"file {_id}: {_written} bytes written, {n} declared");
        // the last part (the only one, empty, for an empty file)
        if (_buffered > 0 || _next == 1) await PutPart(_ctx.Cancelled);
        var call = new V1.FilesFinish { Id = _id, RequestId = _ctx.Request.Id };
        // every part's tag, or none: the platform then lists the parts itself
        if (_etags.All(t => !string.IsNullOrEmpty(t))) call.Etags.AddRange(_etags.Select(t => t!));
        var file = (await _ctx.Syscall(new V1.Syscall { FilesFinish = call })).File ?? throw Files.Unexpected();
        _finished = true;
        _buffer = null;
        return Files.From(file);
    }

    /// <summary>Nothing to do: parts are sent as they fill, the rest by <see cref="Finish"/>.</summary>
    public override void Flush() { }

    public override Task FlushAsync(CancellationToken ct) => Task.CompletedTask;

    public override int Read(byte[] buffer, int offset, int count) => throw new NotSupportedException("a file being written is not read");
    public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException("a file is written from start to end");
    public override void SetLength(long value) => throw new NotSupportedException("a file is written from start to end");

    /// <summary>
    /// Without <see cref="Finish"/>, the file is abandoned: nothing is kept. Disposing does not end the file, so a
    /// <c>StreamWriter</c> over it may be disposed first and <see cref="Finish"/> called after.
    /// </summary>
    protected override void Dispose(bool disposing) => base.Dispose(disposing);
}

/// <summary>
/// The files of <see cref="Ctx.Test"/>: the files syscalls and the store's signed URLs answered in memory, as the
/// platform answers them (parts, pages of URLs, ranged reads). Every other syscall answers <c>unavailable</c>. Tests
/// of the SDK also make URLs expire and connections drop.
/// </summary>
internal sealed class MemoryFiles : ISyscalls
{
    internal const string Origin = "http://files.test/";

    private readonly object _lock = new();
    private readonly Dictionary<string, Stored> _files = [];
    private readonly Dictionary<string, Upload> _uploads = [];
    private int _generation;

    public MemoryFiles() { Http = new HttpClient(new Handler(this)); }

    public HttpClient Http { get; }

    /// <summary>The size of a part (the platform's is at least 16 MiB).</summary>
    public long PartSize { get; init; } = 16L << 20;

    /// <summary>Part URLs per page.</summary>
    public int PageSize { get; init; } = 64;

    /// <summary>A read's connection drops after this many bytes (null: never).</summary>
    public long? DropReadsAfter { get; set; }

    /// <summary>The store's requests, in order: <c>GET o/&lt;id&gt; bytes=5-</c>, <c>PUT u/&lt;id&gt;/2 (16 bytes)</c>.</summary>
    public List<string> Requests { get; } = [];

    /// <summary>Every URL given so far expires: the store refuses it with 403.</summary>
    public void ExpireUrls()
    {
        lock (_lock) _generation++;
    }

    /// <summary>The bytes of a file; null when there is none.</summary>
    public byte[]? Bytes(string id)
    {
        lock (_lock) return _files.TryGetValue(id, out var f) ? f.Bytes : null;
    }

    private sealed record Stored(V1.File File, byte[] Bytes, long CreatedAtMs);

    private sealed class Upload
    {
        public required string Name { get; init; }
        public required string Type { get; init; }
        public long? Size { get; init; }
        public required long PartSize { get; init; }
        public Dictionary<int, byte[]> Parts { get; } = [];

        public uint Count => Size is { } n ? (uint)Math.Max(1, (n + PartSize - 1) / PartSize) : 0;
    }

    public Task<V1.SyscallResult> Call(V1.Syscall call, CancellationToken ct)
    {
        lock (_lock)
        {
            V1.SyscallResult? result = call.CallCase switch
            {
                V1.Syscall.CallOneofCase.FilesCreate => Create(call.FilesCreate),
                V1.Syscall.CallOneofCase.FilesParts => Parts(call.FilesParts),
                V1.Syscall.CallOneofCase.FilesFinish => Finish(call.FilesFinish),
                V1.Syscall.CallOneofCase.FilesOpen => Open(call.FilesOpen),
                V1.Syscall.CallOneofCase.FilesHead => Head(call.FilesHead),
                _ => null,
            };
            return result is null
                ? Task.FromException<V1.SyscallResult>(PluginError.Unavailable("the session with the platform is closed"))
                : Task.FromResult(result);
        }
    }

    public void Invalidate(string topic, string key) { }

    private static V1.SyscallResult Error(string error) => new() { Error = error };

    private long Expiry() => DateTimeOffset.UtcNow.AddHours(1).ToUnixTimeMilliseconds();

    private V1.FilesUpload Page(string id, Upload u, uint from)
    {
        var page = new V1.FilesUpload { Id = id, PartSize = (ulong)u.PartSize, Parts = u.Count, First = from, ExpiresAtMs = Expiry() };
        var last = u.Count == 0 ? from + (uint)PageSize - 1 : Math.Min(u.Count, from + (uint)PageSize - 1);
        for (var n = from; n <= last; n++) page.Urls.Add($"{Origin}u/{id}/{n}?g={_generation}");
        page.Done.AddRange(u.Parts.Keys.Select(k => (uint)k).Order());
        return page;
    }

    private V1.SyscallResult Create(V1.FilesCreate c)
    {
        var name = new string(c.Name.Where(ch => ch is not ('/' or '\\') && !char.IsControl(ch)).ToArray()).TrimStart('.');
        if (name.Length == 0) return Error("bad_request: a file needs a name");
        var id = Guid.CreateVersion7().ToString();
        var u = new Upload
        {
            Name = name,
            Type = c.ContentType.Length == 0 ? "application/octet-stream" : c.ContentType,
            Size = c.HasSize ? (long)c.Size : null,
            PartSize = PartSize,
        };
        _uploads[id] = u;
        return new V1.SyscallResult { FilesUpload = Page(id, u, 1) };
    }

    private V1.SyscallResult Parts(V1.FilesParts p) =>
        _uploads.TryGetValue(p.Id, out var u)
            ? new V1.SyscallResult { FilesUpload = Page(p.Id, u, Math.Max(p.From, 1)) }
            : Error($"not_found: {p.Id} is not a file being written");

    private V1.SyscallResult Finish(V1.FilesFinish f)
    {
        if (!_uploads.TryGetValue(f.Id, out var u)) return Error($"not_found: {f.Id} is not a file being written");
        if (u.Parts.Count == 0) return Error("file_not_uploaded: nothing was written");
        var parts = u.Parts.Keys.Order().ToList();
        if (parts[0] != 1 || parts[^1] != parts.Count) return Error("file_not_uploaded: a part is missing");
        if (f.Etags.Count > 0 && !f.Etags.SequenceEqual(parts.Select(n => Etag(f.Id, n))))
            return Error("bad_request: the parts' tags do not match");
        var bytes = parts.SelectMany(n => u.Parts[n]).ToArray();
        if (u.Size is { } size && bytes.Length != size)
            return Error($"file_size_mismatch: {bytes.Length} bytes written, {size} declared");
        var file = new V1.File { Id = f.Id, Name = u.Name, ContentType = u.Type, Size = (ulong)bytes.Length };
        _files[f.Id] = new Stored(file, bytes, DateTimeOffset.UtcNow.ToUnixTimeMilliseconds());
        _uploads.Remove(f.Id);
        return new V1.SyscallResult { File = file.Clone() };
    }

    private V1.SyscallResult Open(V1.FilesOpen o) =>
        _files.TryGetValue(o.Id, out var f)
            ? new V1.SyscallResult
            {
                FilesRead = new V1.FilesRead { File = f.File.Clone(), Url = $"{Origin}o/{o.Id}?g={_generation}", ExpiresAtMs = Expiry() },
            }
            : Error($"not_found: {o.Id} is not a file of this plugin");

    private V1.SyscallResult Head(V1.FilesHead h) =>
        _files.TryGetValue(h.Id, out var f)
            ? new V1.SyscallResult
            {
                FileInfo = new V1.FileInfo { File = f.File.Clone(), Etag = $"\"{h.Id}\"", CreatedAtMs = f.CreatedAtMs },
            }
            : Error($"not_found: {h.Id} is not a file of this plugin");

    private static string Etag(string id, int part) => $"\"{id}-{part}\"";

    /// <summary>The store: <c>PUT u/&lt;upload&gt;/&lt;part&gt;</c> and <c>GET o/&lt;file&gt;</c> (with <c>Range</c>).</summary>
    private sealed class Handler(MemoryFiles store) : HttpMessageHandler
    {
        protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken ct)
        {
            var uri = request.RequestUri!;
            var path = uri.AbsolutePath.TrimStart('/').Split('/');
            var generation = int.Parse(uri.Query.TrimStart('?').Split('=')[1], CultureInfo.InvariantCulture);
            var body = request.Content is null ? [] : await request.Content.ReadAsByteArrayAsync(ct);
            lock (store._lock)
            {
                store.Requests.Add(request.Method == HttpMethod.Put
                    ? $"PUT {path[0]}/{path[1]}/{path[2]} ({body.Length} bytes)"
                    : $"GET {path[0]}/{path[1]}{(request.Headers.Range is { } r ? $" {r}" : "")}");
                if (generation < store._generation) return new HttpResponseMessage(HttpStatusCode.Forbidden);
                if (request.Method == HttpMethod.Put && path is ["u", var id, var n] && store._uploads.TryGetValue(id, out var u))
                {
                    var part = int.Parse(n, CultureInfo.InvariantCulture);
                    // a part's URL takes exactly its length when the size is known
                    if (u.Size is { } size && body.Length != Math.Min(u.PartSize, size - (part - 1) * u.PartSize))
                        return new HttpResponseMessage(HttpStatusCode.BadRequest);
                    u.Parts[part] = body;
                    var put = new HttpResponseMessage(HttpStatusCode.OK);
                    put.Headers.TryAddWithoutValidation("ETag", Etag(id, part));
                    return put;
                }
                if (request.Method == HttpMethod.Get && path is ["o", var fid] && store._files.TryGetValue(fid, out var f))
                {
                    var from = request.Headers.Range?.Ranges.FirstOrDefault()?.From ?? 0;
                    if (from > 0 && from >= f.Bytes.Length) return new HttpResponseMessage(HttpStatusCode.RequestedRangeNotSatisfiable);
                    var rest = f.Bytes.AsMemory((int)from);
                    var drop = store.DropReadsAfter;
                    return new HttpResponseMessage(from > 0 ? HttpStatusCode.PartialContent : HttpStatusCode.OK)
                    {
                        Content = new StreamContent(new Dropping(rest, drop)),
                    };
                }
                return new HttpResponseMessage(HttpStatusCode.NotFound);
            }
        }
    }

    /// <summary>A body whose connection drops after <c>drop</c> bytes.</summary>
    private sealed class Dropping(ReadOnlyMemory<byte> bytes, long? drop) : Stream
    {
        private int _at;

        public override int Read(byte[] buffer, int offset, int count)
        {
            if (_at >= bytes.Length) return 0;
            if (drop is { } d && _at >= d) throw new IOException("the connection was reset");
            var n = Math.Min(count, bytes.Length - _at);
            if (drop is { } limit) n = (int)Math.Min(n, limit - _at);
            bytes.Span.Slice(_at, n).CopyTo(buffer.AsSpan(offset));
            _at += n;
            return n;
        }

        public override bool CanRead => true;
        public override bool CanSeek => false;
        public override bool CanWrite => false;
        public override long Length => bytes.Length;
        public override long Position { get => _at; set => throw new NotSupportedException(); }
        public override void Flush() { }
        public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();
        public override void SetLength(long value) => throw new NotSupportedException();
        public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();
    }
}
