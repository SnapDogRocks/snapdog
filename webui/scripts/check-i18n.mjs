#!/usr/bin/env node
// Verify every locale in messages/ is COMPLETE against en.json (the source of truth):
//   (1) key parity        — no missing (untranslated) or extra (stale) keys;
//   (2) value completeness — no string left byte-identical to its English source (i.e.
//       never actually translated), unless its key is listed in .i18n-allow.json as an
//       intentional keep (brand/product names, universally-kept technical tokens, or words
//       genuinely spelled the same in the target language).
// Values with no translatable letters once {placeholders} are stripped (pure symbols,
// numbers, emoji, ICU args) are auto-exempted. Fails (exit 1) on any drift so the next-intl
// catalogs can't silently rot. Wired into CI (`npm run i18n:check`) and into `npm run build`
// via the `prebuild` lifecycle hook — so an incomplete catalog also fails the Rust build,
// whose build.rs runs `npm run build`.
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const MESSAGES = join(dirname(fileURLToPath(import.meta.url)), "..", "messages");
const REFERENCE = "en";
const ALLOW_FILE = join(MESSAGES, ".i18n-allow.json");

const flatten = (obj, prefix = "") =>
  Object.entries(obj).flatMap(([k, v]) => {
    const key = prefix ? `${prefix}.${k}` : k;
    return v && typeof v === "object" && !Array.isArray(v) ? flatten(v, key) : [[key, v]];
  });

const loadFlat = (loc) =>
  new Map(flatten(JSON.parse(readFileSync(join(MESSAGES, `${loc}.json`), "utf8"))));

// A value still needs translating only if it contains real letters after simple {arg}
// placeholders are stripped. Pure placeholders / numbers / symbols / emoji never do.
const needsTranslation = (v) =>
  typeof v === "string" && /\p{L}/u.test(v.replace(/\{[A-Za-z0-9_]+\}/g, ""));

const allow = new Set(
  existsSync(ALLOW_FILE) ? JSON.parse(readFileSync(ALLOW_FILE, "utf8")) : []
);

const locales = readdirSync(MESSAGES)
  .filter((f) => f.endsWith(".json") && !f.startsWith("."))
  .map((f) => f.replace(/\.json$/, ""));

const ref = loadFlat(REFERENCE);
const refKeys = [...ref.keys()];
let failed = false;

for (const loc of locales) {
  if (loc === REFERENCE) continue;
  const map = loadFlat(loc);
  const present = new Set(map.keys());
  const missing = refKeys.filter((k) => !present.has(k)).sort();
  const extra = [...present].filter((k) => !ref.has(k)).sort();
  const untranslated = refKeys
    .filter((k) => present.has(k) && !allow.has(k))
    .filter((k) => needsTranslation(ref.get(k)) && map.get(k) === ref.get(k))
    .sort();

  if (missing.length || extra.length || untranslated.length) {
    failed = true;
    console.error(`::error::${loc}.json is out of sync with ${REFERENCE}.json`);
    if (missing.length) console.error(`  missing (${missing.length}): ${missing.join(", ")}`);
    if (extra.length) console.error(`  extra (${extra.length}): ${extra.join(", ")}`);
    if (untranslated.length)
      console.error(`  untranslated / identical to en (${untranslated.length}): ${untranslated.join(", ")}`);
  }
}

if (failed) {
  console.error(
    `\nEvery locale must have exactly the keys of ${REFERENCE}.json and translate every value.\n` +
      `If a value is intentionally identical (brand/technical/same word), add its key to messages/.i18n-allow.json.`
  );
  process.exit(1);
}
console.log(
  `i18n OK: ${locales.length - 1} locale(s) fully match all ${ref.size} keys of ${REFERENCE}.json ` +
    `(${allow.size} intentional keeps).`
);
