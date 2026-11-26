// The plugin process: `node dist/main.js --describe` prints the descriptor (`--form`: with the form laid out by hand);
// otherwise it serves.
import { prefs } from './index.js';

await prefs(process.argv.includes('--form')).serve();
