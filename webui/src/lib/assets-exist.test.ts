import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

// An image that points at a file under /assets which is not shipped loads the app shell
// instead (HTTP 200, text/html) and shows as a broken image. This happened to the cover
// fallback when radio-cover.svg became radio-cover.png.

const SRC = path.resolve(__dirname, "..");
const PUBLIC = path.resolve(__dirname, "../../public");

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = path.join(dir, name);
    if (statSync(full).isDirectory()) return sources(full);
    return /\.(ts|tsx)$/.test(name) && !name.includes(".test.") ? [full] : [];
  });
}

describe("static assets referenced from the source", () => {
  const referenced = new Map<string, string>();
  for (const file of sources(SRC)) {
    for (const match of readFileSync(file, "utf8").matchAll(/["'`](\/assets\/[\w./-]+)["'`]/g)) {
      const asset = match[1];
      if (asset) referenced.set(asset, path.relative(SRC, file));
    }
  }

  it("finds the cover fallback", () => {
    expect(referenced.has("/assets/radio-cover.png")).toBe(true);
  });

  it.each([...referenced])("%s exists in public/ (used in %s)", (asset) => {
    expect(existsSync(path.join(PUBLIC, asset))).toBe(true);
  });
});
