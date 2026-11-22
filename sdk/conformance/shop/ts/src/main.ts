// The plugin process: `node dist/main.js --describe` prints the descriptor; otherwise it serves.
import plugin from './index.js';

await plugin.serve();
