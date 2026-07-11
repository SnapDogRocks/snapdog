#!/usr/bin/env node
// Verify every locale in messages/ has exactly the same keys as en.json (the source of
// truth). Fails on missing keys (untranslated) or extra keys (stale) so the next-intl
// message catalogs can't drift out of completeness. Wired into CI via `npm run i18n:check`.
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const MESSAGES = join(dirname(fileURLToPath(import.meta.url)), "..", "messages");
const REFERENCE = "en";

const flatten = (obj, prefix = "") =>
  Object.entries(obj).flatMap(([k, v]) => {
    const key = prefix ? `${prefix}.${k}` : k;
    return v && typeof v === "object" && !Array.isArray(v) ? flatten(v, key) : [key];
  });

const load = (loc) =>
  new Set(flatten(JSON.parse(readFileSync(join(MESSAGES, `${loc}.json`), "utf8"))));

const locales = readdirSync(MESSAGES)
  .filter((f) => f.endsWith(".json"))
  .map((f) => f.replace(/\.json$/, ""));

const ref = load(REFERENCE);
let failed = false;

for (const loc of locales) {
  if (loc === REFERENCE) continue;
  const keys = load(loc);
  const missing = [...ref].filter((k) => !keys.has(k)).sort();
  const extra = [...keys].filter((k) => !ref.has(k)).sort();
  if (missing.length || extra.length) {
    failed = true;
    console.error(`::error::${loc}.json is out of sync with ${REFERENCE}.json`);
    if (missing.length) console.error(`  missing (${missing.length}): ${missing.join(", ")}`);
    if (extra.length) console.error(`  extra (${extra.length}): ${extra.join(", ")}`);
  }
}

if (failed) {
  console.error(`\nEvery locale must have exactly the keys of ${REFERENCE}.json.`);
  process.exit(1);
}
console.log(`i18n OK: ${locales.length} locales all match ${ref.size} keys in ${REFERENCE}.json`);
