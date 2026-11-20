// Dumps DEFAULT_SETTINGS (src/shared/defaults.ts) and the TS enum lists to a JSON fixture that the Rust
// contract tests compare against. Run: node scripts/export-defaults.mjs
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = new URL('../', import.meta.url);
const defaults = await import(new URL('src/shared/defaults.ts', root));
const types = await import(new URL('src/shared/types.ts', root));
const out = {
  settings: defaults.DEFAULT_SETTINGS,
  moveClasses: types.MOVE_CLASSES,
  tacticTags: types.TACTIC_TAGS,
  events: types.EVT,
};
const dest = fileURLToPath(new URL('src-tauri/tests/fixtures/ts-contract.json', root));
writeFileSync(dest, JSON.stringify(out, null, 2) + '\n');
console.log('wrote', dest);
