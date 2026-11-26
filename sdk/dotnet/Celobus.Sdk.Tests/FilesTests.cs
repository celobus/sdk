// Files: the `File` type in messages and entities (descriptor, JSON), the field options, the kit's file primitives,
// `ctx.Files` over the in-memory store (parts, pages of URLs, expired URLs, dropped connections), files given to agent
// runs, and the `File` alias a plugin compiles with next to `System.IO`.
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Celobus.Sdk.Ui;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.Extensions.Logging.Abstractions;
using U = Celobus.Sdk.Ui.Ui;

namespace Celobus.Sdk.Tests;

[Entity]
public sealed partial record Report(
    Id<Report> Id,
    string Title,
    [MaxSize("1 GiB"), Accept("image/*", "application/pdf")] List<File> Scans,
    File? Cover,
    File Signed);

[Message] public sealed partial record ImportCsvRequest([Accept("text/csv"), MaxSize(1048576)] File File);
[Message] public sealed partial record Imported(long Rows);

[Message] public sealed partial record AttachScansRequest(Guid Id, string Reason, [MaxSize("10 MiB"), Accept("image/*", "application/pdf")] List<File> Scans);
[Message] public sealed partial record ListReportsRequest(string PageToken);
[Message] public sealed partial record ReportRow(Guid Id, string Title, File Signed, List<File> Scans);
[Message] public sealed partial record ListReportsResponse(List<ReportRow> Reports);
[Message] public sealed partial record ReplyRequest(string Text, [MaxSize("2 GiB")] List<File> Files);
[Message] public sealed partial record ReportsParams(string? Search);

public sealed class ImportCsv : Endpoint<ImportCsvRequest, Imported>
{
    public override Task<Imported> Handle(Ctx ctx, ImportCsvRequest req) => Task.FromResult(new Imported(0));
}

public sealed class AttachScans : Endpoint<AttachScansRequest, Imported>
{
    public override void Configure(EndpointOptions e) => e.Ui();
    public override Task<Imported> Handle(Ctx ctx, AttachScansRequest req) => throw new NotSupportedException();
}

public sealed class ListReports : Endpoint<ListReportsRequest, ListReportsResponse>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();
    public override Task<ListReportsResponse> Handle(Ctx ctx, ListReportsRequest req) => throw new NotSupportedException();
}

public sealed class Reply : Endpoint<ReplyRequest, Imported>
{
    public override void Configure(EndpointOptions e) => e.Ui();
    public override Task<Imported> Handle(Ctx ctx, ReplyRequest req) => throw new NotSupportedException();
}

[Message] public sealed partial record NotAFile([MaxSize("1 GiB")] string Name, [MaxSize("lots")] File Big);
[Entity] public sealed partial record Secret(Id<Secret> Id, [Encrypted] File Paper);

public class FilesTests
{
    private static readonly ScreenKey<ReportsParams> ReportsScreen = new("reports");
    private static readonly OverlayKey<ReportsParams> AttachDialog = new("attach");

    private static JsonNode P(string json) => JsonNode.Parse(json)!;

    private static void Same(string expected, JsonNode? actual) =>
        Assert.True(JsonNode.DeepEquals(P(expected), actual), actual?.ToJsonString());

    // --- types, options, descriptor

    [Fact]
    public void FileFieldsAndTheirOptionsArePrinted()
    {
        var plugin = Plugin.Create("reports").Version("0.1.0").Entity<Report>().Endpoint<ImportCsv>();
        Assert.Empty(plugin.Check());
        var d = JsonNode.Parse(plugin.DescriptorJson())!;
        var fields = d["entities"]![0]!["fields"]!;
        Same("""
            {"name":"scans","number":3,"type":"file","repeated":true,"optional":false,"description":"",
             "max_size":1073741824,"accept":["image/*","application/pdf"]}
            """, fields[2]);
        Same("""{"name":"cover","number":4,"type":"file","repeated":false,"optional":true,"description":""}""", fields[3]);
        Same("""{"name":"signed","number":5,"type":"file","repeated":false,"optional":false,"description":""}""", fields[4]);
        // the entity's message keeps the options too
        var messages = d["messages"]!.AsArray();
        Assert.Equal(1L << 30, messages.First(m => (string?)m!["name"] == "Report")!["fields"]![2]!["max_size"]!.GetValue<long>());
        Same("""
            {"name":"file","number":1,"type":"file","repeated":false,"optional":false,"description":"",
             "max_size":1048576,"accept":["text/csv"]}
            """, messages.First(m => (string?)m!["name"] == "ImportCsvRequest")!["fields"]![0]);
    }

    [Fact]
    public void OptionsOnAnotherFieldAndEncryptedFilesAreRefused()
    {
        var errors = Plugin.Create("reports").Version("0.1.0").Message<NotAFile>().Entity<Secret>().Check();
        Assert.Contains(errors, e => e is { Code: "E_FILE_OPTION", Path: "messages.NotAFile.fields.name" });
        Assert.Contains(errors, e => e is { Code: "E_FILE_OPTION", Path: "messages.NotAFile.fields.big.max_size" });
        Assert.Contains(errors, e => e is { Code: "E_FILE_ENCRYPTED", Path: "entities.secret.fields.paper" });
    }

    [Theory]
    [InlineData("1 GiB", 1L << 30)]
    [InlineData("1048576", 1048576L)]
    [InlineData("500 MB", 500_000_000L)]
    [InlineData("1.5 GiB", 3L << 29)]
    [InlineData("2 kib", 2048L)]
    [InlineData(" 10 B ", 10L)]
    [InlineData("10MiB", 10L << 20)]
    [InlineData("0", null)]
    [InlineData("", null)]
    [InlineData("1 XB", null)]
    [InlineData(".5 GiB", null)]
    [InlineData("5. GiB", null)]
    [InlineData("1.2.3 MB", null)]
    [InlineData("-1 MB", null)]
    [InlineData("99999999999 TiB", null)]
    public void SizesReadAsRustReadsThem(string text, long? bytes) => Assert.Equal(bytes, FileSize.Parse(text));

    [Fact]
    public void AFileValueReadsAndWritesItsJson()
    {
        var f = new File("0193", "ads.csv", "text/csv", 2_100_000);
        const string json = """{"id":"0193","name":"ads.csv","type":"text/csv","size":"2100000"}""";
        Assert.Equal(json, CelobusJson.Serialize(f));
        Assert.Equal($$"""{"file":{{json}}}""", CelobusJson.Serialize(new ImportCsvRequest(f)));
        // a size as a number reads the same; unknown keys are left aside
        Assert.Equal(f, CelobusJson.Deserialize<ImportCsvRequest>(
            """{"file":{"id":"0193","name":"ads.csv","type":"text/csv","size":2100000,"url":"https://x"}}""")!.File);
        Assert.Equal(new File("0193", "", "", 7), CelobusJson.Deserialize<File>("""{"id":"0193","size":7}"""));
        // a file without an id is refused, and a single file that is not optional is required
        var noId = Assert.ThrowsAny<JsonException>(() => CelobusJson.Deserialize<ImportCsvRequest>("""{"file":{"name":"x"}}"""));
        Assert.StartsWith("file.id:", noId.Message);
        var missing = Assert.ThrowsAny<JsonException>(() => CelobusJson.Deserialize<ImportCsvRequest>("{}"));
        Assert.StartsWith("file:", missing.Message);
        // an optional one and a list may be left out
        var r = CelobusJson.Deserialize<Report>("""{"id":"r1","title":"T","signed":{"id":"s"}}""")!;
        Assert.Equal((null, 0, "s"), (r.Cover, r.Scans.Count, r.Signed.Id));
    }

    // --- kit

    private static JsonObject Describe(Func<ScreenBuilder<ReportsParams>, Node> layout) =>
        JsonNode.Parse(Plugin.Create("reports").Version("0.1.0")
            .Endpoint<AttachScans>().Endpoint<ListReports>().Endpoint<Reply>()
            .Ui(new PluginUi().Screen(Screen.Build(ReportsScreen, layout)))
            .DescriptorJson())!.AsObject();

    [Fact]
    public void AnUploadInputCarriesItsFieldsOptions()
    {
        var d = Describe(s =>
        {
            s.Overlay(Overlay.Dialog(AttachDialog, o => o.Form(AttachScans.Ref,
                f =>
                {
                    f.Textarea(r => r.Reason, "Why").Required();
                    f.Upload(r => r.Scans, "Scans");
                },
                "Attach",
                (a, _) => a.CallForm(AttachScans.Ref).Close())));
            return U.Text("Reports");
        });
        Same("""
            {"name":"scans","control":"upload","label":"Scans","multiple":true,"max_size":10485760,
             "accept":["image/*","application/pdf"]}
            """, d["ui"]!["screens"]![0]!["overlays"]![0]!["layout"]!["form"]!["sections"]![0]!["inputs"]![1]);
    }

    [Fact]
    public void FileCellsPostsAndTheComposersAttach()
    {
        var d = Describe(s =>
        {
            var reports = s.Query("reports", Query.Call(ListReports.Ref).Rows(r => r.Reports));
            var send = s.Action("send", a => a.Call(Reply.Ref, c => c.Arg(x => x.Text, Form.Text())));
            return U.Stack(
                U.Table(reports).Col(r => r.Title, "Title").File(r => r.Signed, "Signed"),
                U.List(reports, r => r.Title).MetaFile(r => r.Signed),
                U.Stream(reports).Key(r => r.Id).Post(U.Post<ReportRow>().Markdown(r => r.Title).Files(r => r.Scans)),
                U.Composer(send).Tools(ComposerTool.Emoji).Attach((ReplyRequest r) => r.Files).Attach((ReplyRequest r) => r.Files));
        });
        var children = d["ui"]!["screens"]![0]!["layout"]!["stack"]!["children"]!;
        Same("""{"field":"signed","title":"Signed","format":{"file":{}}}""", children[0]!["table"]!["columns"]![1]);
        Same("""{"file":{}}""", children[1]!["list"]!["meta_format"]);
        Assert.Equal("signed", (string?)children[1]!["list"]!["meta"]);
        Assert.Equal("scans", (string?)children[2]!["stream"]!["item"]!["post"]!["files"]);
        Same("""["emoji","attach"]""", children[3]!["composer"]!["tools"]);
        Assert.Equal("files", (string?)children[3]!["composer"]!["files"]);
    }

    // --- ctx.Files

    private static Ctx Context(ISyscalls store, ulong request = 7) => new(
        store, new PluginInfo("reports", "1.0.0"),
        new RequestInfo(request, "Export", DateTimeOffset.UtcNow.AddSeconds(30), null),
        new Caller("reports", null), NullLogger.Instance, CancellationToken.None);

    private static async Task<string> ReadAll(Stream s)
    {
        using var reader = new StreamReader(s);
        return await reader.ReadToEndAsync();
    }

    [Fact]
    public async Task ATestContextWritesAndReadsFilesInMemory()
    {
        var ctx = Ctx.Test("reports", "1.0.0").WithCaller(Caller.TestUser("u1"));
        await using (var w = await ctx.Files.Create("notes.csv", "text/csv"))
        {
            await w.WriteAsync("id,text\n"u8.ToArray());
            w.Write("1,tea\n"u8);
            var file = await w.Finish();
            Assert.Equal(("notes.csv", "text/csv", 14L), (file.Name, file.ContentType, file.Size));

            // the same store after WithRun and AsSystem
            var job = ctx.WithRun("run-1", 1, 3).AsSystem("import");
            Assert.Equal("id,text\n1,tea\n", await ReadAll(await job.Files.Open(file)));
            using var lines = new StreamReader(await job.Files.Open(file, offset: 8));
            Assert.Equal("1,tea", await lines.ReadLineAsync());

            var head = await ctx.Files.Head(file);
            Assert.Equal((file, (Id<Platform.User>?)null), (head.File, head.CreatedBy));
            Assert.NotEmpty(head.Etag);
        }
        // another test context has its own store; an unknown file is 404
        var other = await Assert.ThrowsAsync<PluginError>(() => Ctx.Test("reports", "1.0.0").Files.Open(new File("nope")));
        Assert.Equal((404, "not_found"), (other.Status, other.Code));
        // an empty file is one empty part
        await using var empty = await ctx.Files.Create("empty.txt", "");
        var e = await empty.Finish();
        Assert.Equal(("application/octet-stream", 0L), (e.ContentType, e.Size));
        Assert.Equal("", await ReadAll(await ctx.Files.Open(e)));
    }

    [Fact]
    public async Task AWriterSendsFullPartsWithTheirTagsAndPagesForMoreUrls()
    {
        var store = new MemoryFiles { PartSize = 4, PageSize = 2 };
        var calls = new List<V1.Syscall>();
        var ctx = Context(new Recording(store, calls));
        await using var w = await ctx.Files.Create("a.txt", "text/plain");
        w.Write("abcdefghij"u8); // parts 1 and 2 go now; "ij" waits for Finish
        Assert.Equal(["PUT u/" + w.Id + "/1 (4 bytes)", "PUT u/" + w.Id + "/2 (4 bytes)"], store.Requests);
        var file = await w.Finish();
        Assert.Equal("abcdefghij", Encoding.UTF8.GetString(store.Bytes(file.Id)!));
        Assert.Equal("PUT u/" + w.Id + "/3 (2 bytes)", store.Requests[^1]);
        // part 3 needed the next page; every call carries the request's id; Finish sends the tags in part order
        Assert.Equal([V1.Syscall.CallOneofCase.FilesCreate, V1.Syscall.CallOneofCase.FilesParts, V1.Syscall.CallOneofCase.FilesFinish],
            calls.Select(c => c.CallCase));
        Assert.Equal((3u, 7ul), (calls[1].FilesParts.From, calls[1].FilesParts.RequestId));
        Assert.Equal(7ul, calls[0].FilesCreate.RequestId);
        Assert.False(calls[0].FilesCreate.HasSize);
        Assert.Equal([$"\"{w.Id}-1\"", $"\"{w.Id}-2\"", $"\"{w.Id}-3\""], calls[2].FilesFinish.Etags);
        await Assert.ThrowsAsync<InvalidOperationException>(() => w.Finish());
    }

    [Fact]
    public async Task AStreamWriterOverAFileIsDisposedBeforeFinish()
    {
        var store = new MemoryFiles { PartSize = 4 };
        var ctx = Context(store);
        await using var w = await ctx.Files.Create("notes.csv", "text/csv");
        await using (var text = new StreamWriter(w))
        {
            await text.WriteLineAsync("id,text");
            await text.WriteLineAsync("1,tea");
        }
        var file = await w.Finish();
        Assert.Equal("id,text\n1,tea\n", Encoding.UTF8.GetString(store.Bytes(file.Id)!));
    }

    [Fact]
    public void AFileColumnIsComparedWithNull()
    {
        var tree = Translator.Translate(Ctx.Test("reports", "1.0.0").Query<Report>().Where(r => r.Cover == null).Expression).Tree;
        Assert.Equal(("cover", V1.DbCondition.Types.Op.IsNull), (tree.Filter.Compare.Left.Column.Field, tree.Filter.Compare.Op));
    }

    [Fact]
    public async Task ADeclaredSizeIsReservedAndHeldTo()
    {
        var store = new MemoryFiles { PartSize = 4 };
        var calls = new List<V1.Syscall>();
        var ctx = Context(new Recording(store, calls));
        await using (var w = await ctx.Files.Create("a.txt", "text/plain", size: 6))
        {
            Assert.Equal(6ul, calls[0].FilesCreate.Size);
            var over = Assert.Throws<PluginError>(() => w.Write("abcdefg"u8));
            Assert.Equal((409, "file_size_mismatch"), (over.Status, over.Code));
            await w.WriteAsync("abc"u8.ToArray());
            var under = await Assert.ThrowsAsync<PluginError>(() => w.Finish());
            Assert.Equal((409, "file_size_mismatch"), (under.Status, under.Code));
            await w.WriteAsync("def"u8.ToArray());
            Assert.Equal(6, (await w.Finish()).Size);
        }
        // a writer disposed without Finish leaves no file
        await using (var abandoned = await ctx.Files.Create("b.txt", "text/plain"))
            await abandoned.WriteAsync("abcdef"u8.ToArray());
        Assert.Equal(1, calls.Count(c => c.CallCase == V1.Syscall.CallOneofCase.FilesFinish));
    }

    [Fact]
    public async Task AnExpiredUrlIsAskedForAgain()
    {
        var store = new MemoryFiles { PartSize = 4 };
        var ctx = Context(store);
        await using var w = await ctx.Files.Create("a.txt", "text/plain");
        store.ExpireUrls();
        await w.WriteAsync("abcdef"u8.ToArray()); // part 1: refused (403), signed again, sent
        var file = await w.Finish();
        Assert.Equal("abcdef", Encoding.UTF8.GetString(store.Bytes(file.Id)!));

        await using var r = await ctx.Files.Open(file);
        var two = new byte[2];
        Assert.Equal(2, await r.ReadAsync(two));
        store.ExpireUrls();
        r.Position = 3; // a new GET from byte 3 with an expired URL: opened again
        Assert.Equal("def", await ReadAll(r));
        Assert.Contains(store.Requests, x => x == $"GET o/{file.Id} bytes=3-");
    }

    [Fact]
    public async Task AReadResumesWhereItsConnectionDropped()
    {
        var store = new MemoryFiles();
        var ctx = Context(store);
        await using var w = await ctx.Files.Create("abc.txt", "text/plain");
        await w.WriteAsync("abcdefghijklmnopqrstuvwxyz"u8.ToArray());
        var file = await w.Finish();
        store.DropReadsAfter = 10;
        await using var r = await ctx.Files.Open(file);
        Assert.Equal(26, r.Length);
        Assert.Equal("abcdefghijklmnopqrstuvwxyz", await ReadAll(r));
        Assert.Equal([$"GET o/{file.Id}", $"GET o/{file.Id} bytes=10-", $"GET o/{file.Id} bytes=20-"],
            store.Requests.Where(x => x.StartsWith("GET", StringComparison.Ordinal)));
        // from an offset past the end: nothing to read
        Assert.Equal("", await ReadAll(await ctx.Files.Open(file, offset: 26)));
    }

    [Fact]
    public void FileRefusalsKeepTheirStatus()
    {
        foreach (var (error, status) in new[]
                 {
                     ("file_gone: collected", 410), ("quota_exceeded: full", 507), ("file_too_large: big", 413),
                     ("read_only: no", 409), ("file_size_mismatch: 3", 409), ("file_unknown: x", 409), ("not_found: x", 404),
                 })
        {
            var e = Ctx.SyscallFailed(error);
            Assert.Equal((status, error.Split(':')[0]), (e.Status, e.Code));
        }
    }

    // --- agents

    [Fact]
    public async Task FilesGoWithARunAndItsFollowUps()
    {
        var syscalls = new FakeSyscalls(call => call.CallCase == V1.Syscall.CallOneofCase.AgentSend
            ? new V1.SyscallResult { AgentInput = new V1.AgentRunInput { Seq = 4 } }
            : new V1.SyscallResult { AgentRun = new V1.AgentRun { Id = "run-1", Status = "starting", Agent = "assistant", StartedBy = new V1.AgentRunActor { UserId = "u-1" } } });
        var ctx = new Ctx(syscalls, new PluginInfo("tests", "1.0.0"),
            new RequestInfo(77, "Ask", DateTimeOffset.UtcNow.AddSeconds(30), null),
            new Caller("tests", null), NullLogger.Instance, CancellationToken.None);
        var csv = new File("f1", "ads.csv", "text/csv", 2_100_000);
        await ctx.Agents.Run("assistant", "look", new AgentRunOptions { Files = [csv] });
        Assert.Equal(4, await ctx.Agents.Send("run-1", "and this", [new File("f2")]));
        await ctx.Agents.Send("run-1", "no files");
        var start = syscalls.Calls[0].AgentRun.Files.Single();
        Assert.Equal(("f1", "ads.csv", "text/csv", 2_100_000ul), (start.Id, start.Name, start.ContentType, start.Size));
        Assert.Equal("f2", syscalls.Calls[1].AgentSend.Files.Single().Id);
        Assert.Empty(syscalls.Calls[2].AgentSend.Files);
    }

    // --- the alias

    private static IEnumerable<Diagnostic> Compile(string source)
    {
        var tpa = ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!).Split(Path.PathSeparator);
        var refs = tpa.Select(p => MetadataReference.CreateFromFile(p)).ToList();
        refs.Add(MetadataReference.CreateFromFile(typeof(Ctx).Assembly.Location));
        return CSharpCompilation.Create("plugin", [CSharpSyntaxTree.ParseText(source)], refs,
                new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary, nullableContextOptions: NullableContextOptions.Enable))
            .GetDiagnostics().Where(d => d.Severity == DiagnosticSeverity.Error);
    }

    private const string PluginSource = """
        using System.IO;
        using System.Threading.Tasks;
        using Celobus.Sdk;

        namespace Notes;

        [Message] public sealed partial record ImportCsv([Accept("text/csv")] File File);

        public static class Import
        {
            public static async Task<string> Run(Ctx ctx, ImportCsv p)
            {
                using var reader = new StreamReader(await ctx.Files.Open(p.File));
                return await reader.ReadToEndAsync() + System.IO.File.Exists("x");
            }
        }
        """;

    [Fact]
    public void APluginNamesFileNextToSystemIoThroughTheAlias()
    {
        // without the alias, File names two types; with it (what the package declares), the SDK's
        Assert.Contains(Compile("global using System.IO;\n" + PluginSource), d => d.Id == "CS0104");
        Assert.Empty(Compile("global using System.IO;\nglobal using File = Celobus.Sdk.File;\n" + PluginSource));
    }

    /// <summary>The store's syscalls, recorded.</summary>
    private sealed class Recording(MemoryFiles store, List<V1.Syscall> calls) : ISyscalls
    {
        public HttpClient Http => store.Http;

        public Task<V1.SyscallResult> Call(V1.Syscall call, CancellationToken ct)
        {
            calls.Add(call.Clone());
            return store.Call(call, ct);
        }

        public void Invalidate(string topic, string key) { }
    }
}
