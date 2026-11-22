# Celobus.Sdk

The [Celobus](https://celobus.com) plugin SDK for .NET: a plugin's identity, endpoints, entities, screens, agents
and jobs declared in code, the descriptor it prints for `--describe`, and its session with the platform. .NET 10.
The package carries its analyzers: a query the SDK cannot translate is a build error (`CELOBUS060`).

```sh
dotnet add package Celobus.Sdk
```

```csharp
using Celobus.Sdk;

[Message] public sealed partial record HelloRequest(string Name);
[Message] public sealed partial record HelloReply(string Text);

public static class Roles
{
    public static readonly Role Clerk = new("clerk") { Title = "Order clerk" };
}

public sealed class Hello : Endpoint<HelloRequest, HelloReply>
{
    public override void Configure(EndpointOptions e) => e.Describe("Greets.").Roles(Roles.Clerk);

    public override Task<HelloReply> Handle(Ctx ctx, HelloRequest req) =>
        Task.FromResult(new HelloReply($"Hello, {req.Name}!"));
}
```

```csharp
// Program.cs: `--describe` prints the descriptor; otherwise it serves. The plugin's version is the assembly's.
await Plugin.Create("hello").Title("Hello").Role(Roles.Clerk).Endpoint<Hello>().Serve();
```

The SDKs for Rust and TypeScript and the plugin contract: <https://github.com/celobus/sdk>.

Source-available under the PolyForm Shield License 1.0.0 (`LICENSE`).
