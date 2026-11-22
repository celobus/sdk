# celobus-sdk

The [Celobus](https://celobus.com) plugin SDK for Rust: `#[derive(Message)]` and `#[derive(Entity)]` on a plugin's
types, builder calls for its identity, endpoints, entities, screens, agents and jobs, and `async fn(Ctx, Request) ->
Result<Reply>` handlers. `serve()` prints the plugin's descriptor with `--describe`; otherwise it connects to the
platform and serves the endpoints.

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

The SDKs for TypeScript and C# and the plugin contract: <https://github.com/celobus/sdk>.

Source-available under the PolyForm Shield License 1.0.0 (`LICENSE`).
