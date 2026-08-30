// SPDX-License-Identifier: GPL-3.0-only
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test, { after } from "node:test";

process.env.SNAPDOG_UPDATER_TEST = "true";
process.env.UPDATER_TOKEN = "a".repeat(64);
const testDirectory = mkdtempSync(join(tmpdir(), "snapdog-updater-control-"));
const progressFile = join(testDirectory, "progress.json");
process.env.PROGRESS_FILE = progressFile;
after(() => rmSync(testDirectory, { recursive: true, force: true }));
const { equalToken, publicStatus, startProgressJournal } = await import("./control.mjs");
const {
  fetchReleaseVersion,
  newer,
  reached,
  releaseDecision,
  retryAtFromHeaders,
  stableServerReleaseTag,
  validateConfig,
  zonedClock,
} = await import("./core.mjs");

test("resets a completed journal synchronously when a new update starts", () => {
  // The path is created by mkdtemp above and cannot escape the test directory.
  // eslint-disable-next-line security/detect-non-literal-fs-filename
  writeFileSync(
    progressFile,
    JSON.stringify({
      phase: "done",
      detail: null,
      at: "2026-08-15T08:00:10.000Z",
      startedAt: "2026-08-15T08:00:00.000Z",
    })
  );

  assert.equal(startProgressJournal("v1.12.0", new Date("2026-08-16T09:00:00.000Z")), true);
  // eslint-disable-next-line security/detect-non-literal-fs-filename
  assert.deepEqual(JSON.parse(readFileSync(progressFile, "utf8")), {
    phase: "verifying",
    detail: "v1.12.0",
    at: "2026-08-16T09:00:00.000Z",
    startedAt: "2026-08-16T09:00:00.000Z",
    failedPhase: null,
    rollbackAttempted: false,
  });
});

test("compares semantic release versions without allowing downgrades", () => {
  assert.equal(newer("1.8.1", "v1.8.2"), true);
  assert.equal(newer("v1.9.9", "v2.0.0"), true);
  assert.equal(newer("v2.0.0", "v1.99.99"), false);
  assert.equal(newer("v1.8.2", "v1.8.2"), false);
  assert.equal(newer("dev", "v1.8.2"), true);
  assert.equal(newer("v1.8.2", "client-v1.3.2"), false);
});

test("confirms an update only from the version actually running", () => {
  assert.equal(reached("v1.10.5", "v1.10.5"), true);
  assert.equal(reached("v1.10.6", "v1.10.5"), true);
  assert.equal(reached("v1.10.4", "v1.10.5"), false);
  assert.equal(reached(null, "v1.10.5"), false);
});

test("does not offer a release until every signed container artifact is ready", () => {
  assert.deepEqual(releaseDecision("v1.10.3", "v1.10.4", false), {
    state: "preparing",
    updateAvailable: false,
  });
  assert.deepEqual(releaseDecision("v1.10.3", "v1.10.4", true), {
    state: "available",
    updateAvailable: true,
  });
  assert.deepEqual(releaseDecision("v1.10.4", "v1.10.4", false), {
    state: "current",
    updateAvailable: false,
  });
});

test("requires an exact control token", () => {
  assert.equal(equalToken("a".repeat(64)), true);
  assert.equal(equalToken("a".repeat(63)), false);
  assert.equal(equalToken("b".repeat(64)), false);
});

test("validates and evaluates timezone-aware maintenance schedules", () => {
  assert.equal(
    validateConfig({ mode: "automatic", maintenanceTime: "02:30", timezone: "Europe/Berlin" }),
    true
  );
  assert.equal(validateConfig({ mode: "manual", maintenanceTime: "23:59", timezone: "UTC" }), true);
  assert.equal(
    validateConfig({ mode: "automatic", maintenanceTime: "24:00", timezone: "UTC" }),
    false
  );
  assert.equal(
    validateConfig({ mode: "automatic", maintenanceTime: "02:30", timezone: "Not/AZone" }),
    false
  );
  assert.deepEqual(zonedClock(new Date("2026-08-11T00:30:00Z"), "Europe/Berlin"), {
    date: "2026-08-11",
    time: "02:30",
  });
});

test("reports its own image version so a stale updater is visible", () => {
  const status = publicStatus();
  // Built without the ARG in tests, so the field exists but is null — exactly
  // what an older updater looks like to the server, which must tolerate it.
  assert.ok("updaterVersion" in status, "status must always carry updaterVersion");
  assert.ok("updaterUpdateAvailable" in status, "status must always carry updaterUpdateAvailable");
  assert.equal(status.releaseCheckStatus, "ok");
  assert.equal(status.releaseCheckError, null);
  assert.equal(status.releaseCheckRetryAt, null);
  assert.equal(status.updaterSelfUpdateCapable, true);
  assert.equal(status.updaterSelfUpdateEnabled, true);
  assert.equal(status.updaterUpdateAvailable, false, "must not claim an update before a check ran");
});

test("treats an unknown own version as older than any release", () => {
  // A build without a baked version must still be reported as behind rather
  // than silently current.
  assert.equal(newer(null, "v1.9.6"), true);
  assert.equal(newer("0.0.0-dev", "v1.9.6"), true);
  assert.equal(newer("v1.9.6", "v1.9.6"), false);
  assert.equal(newer("v1.9.7", "v1.9.6"), false);
});

test("selects the newest stable server release without being masked by other components", () => {
  assert.equal(
    stableServerReleaseTag([
      { tag_name: "client-v1.4.2", draft: false, prerelease: false },
      { tag_name: "v1.10.1", draft: false, prerelease: false },
      { tag_name: "v1.10.2", draft: false, prerelease: false },
      { tag_name: "v2.0.0", draft: false, prerelease: true },
      { tag_name: "v9.0.0", draft: true, prerelease: false },
    ]),
    "v1.10.2"
  );
  assert.equal(
    stableServerReleaseTag([{ tag_name: "client-v1.4.2", draft: false, prerelease: false }]),
    null
  );
  // Keep custom RELEASE_API endpoints pointing at one release compatible.
  assert.equal(
    stableServerReleaseTag({ tag_name: "v1.10.2", draft: false, prerelease: false }),
    "v1.10.2"
  );
});

test("retries transient GitHub failures and accepts the next valid release", async () => {
  const responses = [
    new Response("gateway timeout", { status: 504 }),
    Response.json({ tag_name: "v1.13.0", draft: false, prerelease: false }),
  ];
  const delays = [];
  const tag = await fetchReleaseVersion(
    "https://example.test/releases/latest",
    async () => responses.shift(),
    async (delay) => delays.push(delay)
  );
  assert.equal(tag, "v1.13.0");
  assert.deepEqual(delays, [500]);
});

test("rejects an implausibly empty release response after bounded retries", async () => {
  let attempts = 0;
  await assert.rejects(
    fetchReleaseVersion(
      "https://example.test/releases/latest",
      async () => {
        attempts += 1;
        return Response.json([]);
      },
      async () => undefined
    ),
    (error) => error?.code === "invalid-response"
  );
  assert.equal(attempts, 3);
});

test("honors GitHub rate-limit reset metadata without busy retrying", async () => {
  const now = Date.parse("2026-08-17T12:00:00.000Z");
  const retryAt = retryAtFromHeaders(
    new Headers({ "retry-after": "120", "x-ratelimit-reset": "1" }),
    now
  );
  assert.equal(retryAt, "2026-08-17T12:02:00.000Z");
  let attempts = 0;
  await assert.rejects(
    fetchReleaseVersion(
      "https://example.test/releases/latest",
      async () => {
        attempts += 1;
        return new Response(null, { status: 429, headers: { "retry-after": "120" } });
      },
      async () => undefined,
      () => now
    ),
    (error) => error?.code === "rate-limited" && error?.retryAt === retryAt
  );
  assert.equal(attempts, 1);
});
