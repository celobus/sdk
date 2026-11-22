using Celobus.Sdk;

namespace Hello;

[Message]
public enum Mood { Happy, Sad }

[Message] public sealed partial record HelloRequest(string Name, Mood? Mood = null);
[Message] public sealed partial record HelloReply(string Text, long Count);

/// <summary>Greets, and counts the greetings of that name: a row each, in a transaction with its count.</summary>
public sealed class Hello : Endpoint<HelloRequest, HelloReply>
{
    public override void Configure(EndpointOptions e) => e
        .Describe("Greets.")
        .Roles(Roles.Clerk, Roles.Manager);

    public override async Task<HelloReply> Handle(Ctx ctx, HelloRequest req)
    {
        if (string.IsNullOrWhiteSpace(req.Name)) throw PluginError.Invalid("whom to greet?").Field("name", "required");
        var count = await ctx.Transaction(async tx =>
        {
            var greetings = tx.Db<Greeting>();
            await greetings.Create(new Greeting(default, req.Name, null));
            return await greetings.Count(Filter<Greeting>.Eq(g => g.Name, req.Name));
        });
        var text = req.Mood == Mood.Sad ? $"Cheer up, {req.Name}." : $"Hello, {req.Name}!";
        return new HelloReply(text, count);
    }
}
