// The SDK conformance plugin `prefs`: a plugin's settings, the same in Rust, TypeScript and C#. Its settings hold every
// type a setting takes (text, a number of each kind, a decimal, a date, a bool, an enum, lists of enums, texts and
// members, a member, a secret) and every validator, in sections. `--describe` prints the form derived from them and must
// equal ../descriptor.json; with `--form` the plugin lays the form out by hand and checks each change (OnChange), and
// `--describe` must equal ../descriptor-form.json (scripts/sdk-conformance.sh).
using System.ComponentModel;
using Celobus.Sdk;

namespace Prefs;

[Message]
public enum Tone { Calm, LoudAndClear }

[Message]
public enum Channel { Email, Chat, Push }

[Message(Kind = MessageKind.Settings)]
[Describe("What the owners and admins of a workspace set for the plugin.")]
public sealed record PrefsSettings
{
    [Describe("After this many hours without messages a thread no longer sticks to the agent that answered in it.")]
    [Label("Thread stays with its agent, hours"), Range(1, 48), Section("Agents")]
    public int ThreadWarmHours { get; init; } = 2;

    [Describe("The longest message a member may send.")]
    [Range(100, 20000), ValidationMessage("From 100 to 20,000 characters"), Section("Agents")]
    public long MessageMaxChars { get; init; } = 20000;

    [Range(0, 1)]
    public double SampleRate { get; init; } = 0.5;

    [Range("0", "100.00")]
    public decimal Fee { get; init; } = 12.50m;

    [Range("2020-01-01", "2030-12-31")]
    public DateOnly? StartsOn { get; init; }

    public bool Enabled { get; init; } = true;

    // the enum's first value: a default the initial value cannot say
    [DefaultValue(Tone.Calm)]
    public Tone Tone { get; init; }

    [Label("Notify by"), MinItems(1), MaxItems(2)]
    public List<Channel> Channels { get; init; } = [Channel.Email];

    [MaxItems(5), Placeholder("One per line")]
    public List<string> Tags { get; init; } = [];

    [MaxItems(3)]
    public List<Id<Celobus.Sdk.Platform.User>> Watchers { get; init; } = [];

    public Id<Celobus.Sdk.Platform.User>? Owner { get; init; }

    [Describe("The first message of a thread.")]
    [Multiline, Pattern("^[A-Z]"), MinLength(1), MaxLength(500)]
    public required string Greeting { get; init; }

    [Label("Webhook URL"), Url, Placeholder("https://"), Section("Integrations")]
    public string? WebhookUrl { get; init; }

    [Email, Section("Integrations")]
    public string? Contact { get; init; }

    [MinLength(8), MaxLength(200), Section("Integrations")]
    public Secret? WebhookToken { get; init; }
}

[Message] public sealed record HoursRequest;
[Message] public sealed record HoursReply(int Hours, Tone Tone);

/// <summary>How long a thread stays with its agent, as the settings say.</summary>
public sealed class Hours : Endpoint<HoursRequest, HoursReply>
{
    public override void Configure(EndpointOptions e) => e.Describe("How long a thread stays with its agent.");

    public override Task<HoursReply> Handle(Ctx ctx, HoursRequest req)
    {
        var s = ctx.Settings<PrefsSettings>();
        return Task.FromResult(new HoursReply(s.ThreadWarmHours, s.Tone));
    }
}

public static class PrefsPlugin
{
    /// <summary>The plugin; <paramref name="form"/>: the form laid out by hand, and each change checked.</summary>
    public static Plugin Define(bool form) => Plugin.Create("prefs")
        .Endpoint<Hours>()
        .Settings<PrefsSettings>(form ? s => s.OnChange(Check).Form(f =>
        {
            f.Section("Agents");
            f.Number(x => x.ThreadWarmHours, "Hours").Help("How long a thread stays with its agent");
            f.Number(x => x.MessageMaxChars, "Longest message");
            f.Section("Behaviour");
            f.Number(x => x.SampleRate, "Sample rate");
            f.Number(x => x.Fee, "Fee");
            f.Date(x => x.StartsOn, "Starts on");
            f.Checkbox(x => x.Enabled, "Enabled");
            f.Radio(x => x.Tone, "Tone");
            f.MultiSelect(x => x.Channels, "Channels");
            f.Textarea(x => x.Tags, "Tags");
            f.Member(x => x.Watchers, "Watchers");
            f.Member(x => x.Owner, "Owner");
            f.Textarea(x => x.Greeting, "Greeting").Placeholder("Hello!");
            f.Section("Integrations");
            f.Text(x => x.WebhookUrl, "URL").Required();
            f.Text(x => x.Contact, "Contact");
            f.Secret(x => x.WebhookToken, "Token");
        }) : null);

    /// <summary>A token needs the URL it is for.</summary>
    private static Task Check(Ctx ctx, SettingsChange<PrefsSettings> change)
    {
        if (change.Current.WebhookToken is not null && change.Current.WebhookUrl is null)
            throw PluginError.Invalid("a token needs a URL").Field("webhook_url", "Set the URL the token is for");
        return Task.CompletedTask;
    }
}
