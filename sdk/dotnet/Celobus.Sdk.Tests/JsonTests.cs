using System.Text.Json;
using Platform = Celobus.Sdk.Platform;

namespace Celobus.Sdk.Tests;

public class JsonTests
{
    private static readonly Sample Full = new(
        Text: "hi <there>",
        Big: 9007199254740993,
        Small: -3,
        Ratio: 0.5,
        Flag: true,
        Amount: 12.50m,
        Uuid: Guid.Parse("0192d1c4-1b7e-7f00-8000-000000000001"),
        At: new DateTimeOffset(2024, 5, 1, 12, 20, 30, 123, TimeSpan.FromHours(2)),
        Day: new DateOnly(2024, 2, 29),
        Data: [1, 2, 3],
        Status: Status.OnHold,
        Maybe: null,
        Lines: [new Line("CHAI", 2)],
        Single: null,
        ThingId: new Id<Thing>(42),
        Owner: new Id<Platform.User>("u-1"),
        Note: null,
        Numbers: [1, -2]);

    private const string FullJson = """
        {"text":"hi <there>","big":"9007199254740993","small":-3,"ratio":0.5,"flag":true,"amount":"12.50","uuid":"0192d1c4-1b7e-7f00-8000-000000000001","at":"2024-05-01T10:20:30.123Z","day":"2024-02-29","data":"AQID","status":"on_hold","lines":[{"sku":"CHAI","quantity":2}],"thingId":"42","owner":"u-1","numbers":["1","-2"]}
        """;

    [Fact]
    public void WritesTheProtobufJsonMapping() => Assert.Equal(FullJson, CelobusJson.Serialize(Full));

    [Fact]
    public void ReadsWhatItWrites()
    {
        var back = CelobusJson.Deserialize<Sample>(FullJson)!;
        Assert.Equal(CelobusJson.Serialize(Full), CelobusJson.Serialize(back));
        Assert.Equal(42, back.ThingId.ToInt64());
        Assert.Equal(Full.At, back.At);
    }

    [Fact]
    public void MissingFieldsTakeProtobufDefaults()
    {
        var s = CelobusJson.Deserialize<Sample>("{}")!;
        Assert.Equal("", s.Text);
        Assert.Equal(0, s.Big);
        Assert.Equal(0m, s.Amount);
        Assert.Equal(DateTimeOffset.UnixEpoch, s.At);
        Assert.Empty(s.Data);
        Assert.Equal(Status.Placed, s.Status);
        Assert.Null(s.Maybe);
        Assert.Empty(s.Lines);
        Assert.Null(s.Single);
        Assert.True(s.ThingId.IsEmpty);
        Assert.Null(s.Owner);
        Assert.Null(s.Note);
        Assert.Empty(s.Numbers);
    }

    [Fact]
    public void AcceptsSnakeNamesNumbersAndUnknownFields()
    {
        var s = CelobusJson.Deserialize<Sample>(
            """{"thing_id":7,"big":12,"amount":1.25,"status":"OnHold","unknown":{"a":[1]},"note":null,"maybe":"placed"}""")!;
        Assert.Equal("7", s.ThingId.Value);
        Assert.Equal(12, s.Big);
        Assert.Equal(1.25m, s.Amount);
        Assert.Equal(Status.OnHold, s.Status);
        Assert.Equal(Status.Placed, s.Maybe);
        Assert.Null(s.Note);
    }

    [Fact]
    public void RefusesAnUnknownEnumValue() =>
        Assert.Equal("status", Assert.ThrowsAny<JsonException>(() => CelobusJson.Deserialize<Sample>("""{"status":"lost"}""")) is FieldJsonException f ? f.FieldPath : null);

    [Fact]
    public void InitOnlyMessages()
    {
        var p = CelobusJson.Deserialize<Props>("""{"alpha":3,"zeta":"z"}""")!;
        Assert.Equal("z", p.Zeta);
        Assert.Equal(3, p.Alpha);
        Assert.Equal("""{"zeta":"z","alpha":3}""", CelobusJson.Serialize(p));
    }

    [Theory]
    [InlineData(0, "2024-01-02T03:04:05Z")]
    [InlineData(1_230_000, "2024-01-02T03:04:05.123Z")]
    [InlineData(1_234_560, "2024-01-02T03:04:05.123456Z")]
    [InlineData(1_234_567, "2024-01-02T03:04:05.123456700Z")]
    public void TimestampsUseZeroThreeSixOrNineDigits(long ticks, string expected)
    {
        var at = new DateTimeOffset(2024, 1, 2, 3, 4, 5, TimeSpan.Zero).AddTicks(ticks);
        Assert.Equal(expected, TimestampConverter.Format(at));
        Assert.Equal(at, JsonSerializer.Deserialize<DateTimeOffset>($"\"{expected}\"", CelobusJson.Options));
    }

    [Fact]
    public void AnonymousInputsUseTheSameMapping() =>
        Assert.Equal("""{"thingId":"5","count":"3","price":"1.10"}""",
            CelobusJson.Serialize(new { ThingId = new Id<Thing>(5), Count = 3L, Price = 1.10m }));

    [Fact]
    public void OmitDefaultsLeavesOutEmptyFields()
    {
        var empty = Trimmed.Empty;
        // an enum, an optional field with a value and a message without [OmitDefaults] are written; everything else
        // at its default is not
        Assert.Equal("""{"maybe":0,"status":"placed","line":{"sku":"","quantity":0}}""", CelobusJson.Serialize(empty));
        Assert.Equal(CelobusJson.Serialize(empty), CelobusJson.Serialize(CelobusJson.Deserialize<Trimmed>(CelobusJson.Serialize(empty))!));

        var full = empty with
        {
            Text = "hi", Count = 2, Big = 3, Weight = 0.5, Flag = true, Tags = ["a"], Face = new Face("u-1", false),
            Faces = [new Face("", false)],
        };
        Assert.Equal(
            """{"text":"hi","count":2,"big":"3","weight":0.5,"flag":true,"tags":["a"],"face":{"id":"u-1"},"faces":[{}],"maybe":0,"status":"placed","line":{"sku":"","quantity":0}}""",
            CelobusJson.Serialize(full));
        var back = CelobusJson.Deserialize<Trimmed>(CelobusJson.Serialize(full))!;
        Assert.Equal(CelobusJson.Serialize(full), CelobusJson.Serialize(back));
        Assert.Equal("u-1", back.Face.Id);
        // an absent field reads as its default
        Assert.Equal("", back.Faces[0].Id);
    }

    [Fact]
    public void OmitDefaultsIsNotForEntities() =>
        Assert.Equal("""{"id":"r-1","name":""}""", CelobusJson.Serialize(new TrimmedRow(new Id<TrimmedRow>("r-1"), "")));
}

[Entity, OmitDefaults] public sealed record TrimmedRow(Id<TrimmedRow> Id, string Name);

/// <summary>A face of a trimmed view: its empty fields are left out.</summary>
[Message, OmitDefaults] public sealed record Face(string Id, bool Agent);

[Message, OmitDefaults]
public sealed record Trimmed(
    string Text, int Count, long Big, double Weight, bool Flag, IReadOnlyList<string> Tags, Face Face,
    IReadOnlyList<Face> Faces, int? Maybe, Status Status, Line Line)
{
    public static Trimmed Empty { get; } = new("", 0, 0, 0, false, [], new Face("", false), [], 0, Status.Placed, new Line("", 0));
}
