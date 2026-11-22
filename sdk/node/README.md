# @celobus/sdk

The [Celobus](https://celobus.com) plugin SDK for Node.js (TypeScript): a plugin's identity, endpoints, entities,
screens, agents and jobs declared in code, the descriptor it prints for `--describe`, and its session with the
platform. Node.js 22 or later.

```sh
npm install @celobus/sdk
```

```ts
// src/index.ts
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
```

```ts
// src/main.ts: `node dist/main.js --describe` prints the descriptor; otherwise it serves
import plugin from './index.js';

await plugin.serve();
```

The SDKs for Rust and C# and the plugin contract: <https://github.com/celobus/sdk>.

Source-available under the PolyForm Shield License 1.0.0 (`LICENSE`).
