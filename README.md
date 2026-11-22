# Celobus plugin SDKs

The SDKs for writing [Celobus](https://celobus.com) plugins in Rust, TypeScript and C#, and the plugin contract
they share. A plugin declares itself in code: its roles, endpoints, entities, screens, agents and jobs. Run with
`--describe`, it prints its descriptor; otherwise it connects to the platform and serves its endpoints. Plugins are
built, checked and published with the `celobus` CLI.

| Language | Package | Install | Source |
|---|---|---|---|
| Rust | [`celobus-sdk`](https://crates.io/crates/celobus-sdk) | `cargo add celobus-sdk` | `crates/sdk`, with `sdk-derive`, `proto`, `manifest`, `descriptor` |
| TypeScript | [`@celobus/sdk`](https://www.npmjs.com/package/@celobus/sdk) | `npm install @celobus/sdk` | `sdk/node` |
| C# | [`Celobus.Sdk`](https://www.nuget.org/packages/Celobus.Sdk) | `dotnet add package Celobus.Sdk` | `sdk/dotnet` |

Requirements: a current stable Rust (edition 2024), Node.js 22 or later, .NET 10.

## A plugin in each language

Rust:

```rust
use celobus_sdk::prelude::*;

pub const CLERK: Role = Role::new("clerk").title("Order clerk");

#[derive(Message, Clone, Debug)]
pub struct HelloRequest { pub name: String }
#[derive(Message, Clone, Debug)]
pub struct HelloReply { pub text: String }

async fn hello(_ctx: Ctx, req: HelloRequest) -> Result<HelloReply> {
    Ok(HelloReply { text: format!("Hello, {}!", req.name) })
}

fn main() {
    Plugin::new("hello", env!("CARGO_PKG_VERSION"))
        .title("Hello")
        .role(CLERK)
        .endpoint(Endpoint::new(hello).roles([CLERK]).describe("Greets."))
        .serve()
}
```

TypeScript:

```ts
import { endpoint, plugin, role, t } from '@celobus/sdk';

const clerk = role({ title: 'Order clerk' });

export default plugin({
  id: 'hello', title: 'Hello', roles: { clerk },
  endpoints: {
    hello: endpoint({
      input: t.message('HelloRequest', { name: t.text() }),
      output: t.message('HelloReply', { text: t.text() }),
      roles: [clerk],
      async handler(ctx, { name }) { return { text: `Hello, ${name}!` }; },
    }),
  },
});
// src/main.ts: import plugin from './index.js'; await plugin.serve();
```

C#:

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

// Program.cs
await Plugin.Create("hello").Title("Hello").Role(Roles.Clerk).Endpoint<Hello>().Serve();
```

## The repository

- `proto/plugin.proto`: the plugin contract, the session between a plugin and the platform.
- `crates/`: the Rust SDK (`sdk`, `sdk-derive`) and the crates it is built on: the generated contract (`proto`), the
  plugin manifest (`manifest`) and the descriptor with its validation (`descriptor`).
- `sdk/node`, `sdk/dotnet`: the TypeScript and C# SDKs. The C# package carries its analyzers.
- `sdk/conformance`: the same plugins written with each SDK; each must describe itself identically.
  `scripts/sdk-conformance.sh` builds them and compares their descriptors (it needs `jq`, and skips a language
  whose toolchain is missing).

Tests: `cargo test --workspace`, `(cd sdk/node && npm ci && npm run build && npm test)`, `dotnet test sdk/dotnet`.

## License

Source-available under the [PolyForm Shield License 1.0.0](LICENSE): you may use, change and distribute the SDKs
for any purpose, commercial included, except to provide a product that competes with Celobus.

This repository is a read-only mirror of the SDKs as they are released. Issues are welcome; pull requests are not
merged here.
