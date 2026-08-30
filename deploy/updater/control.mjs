// SPDX-License-Identifier: GPL-3.0-only
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { timingSafeEqual } from "node:crypto";
import { mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import {
  ReleaseCheckError,
  fetchReleaseVersion,
  newer,
  reached,
  releaseDecision,
  validateConfig,
  zonedClock,
} from "./core.mjs";

const token = process.env.UPDATER_TOKEN ?? "";
const intervalSeconds = Number(process.env.POLL_INTERVAL_SECONDS ?? 900);
const operationTimeoutMs = Number(process.env.UPDATE_TIMEOUT_SECONDS ?? 900) * 1000;
const port = Number(process.env.CONTROL_PORT ?? 8080);
// Server releases explicitly own GitHub's Latest badge; other component workflows set
// make_latest=false. The single-release endpoint is therefore authoritative and
// avoids downloading the repository's increasingly large release collection.
const releaseApi =
  process.env.RELEASE_API ?? "https://api.github.com/repos/SnapDogRocks/snapdog/releases/latest";
const serverImageRepository = process.env.IMAGE_REPOSITORY ?? "ghcr.io/snapdogrocks/snapdog";
const updaterImageRepository =
  process.env.UPDATER_IMAGE_REPOSITORY ?? "ghcr.io/snapdogrocks/snapdog-updater";
const cosignIssuer = "https://token.actions.githubusercontent.com";
const serverIdentity =
  "^https://github\\.com/SnapDogRocks/snapdog/\\.github/workflows/release\\.yml@refs/tags/v[0-9]+\\.[0-9]+\\.[0-9]+$";
const updaterIdentity =
  "^https://github\\.com/SnapDogRocks/snapdog/\\.github/workflows/release\\.yml@refs/tags/v[0-9]+\\.[0-9]+\\.[0-9]+$";
/* Own image version, baked in at build time (Dockerfile ARG UPDATER_VERSION).
 * Current updaters can hand their replacement to a detached, health-checked
 * helper. Older updaters omit these fields, which lets the UI show the one-time
 * manual bootstrap instead of prescribing manual maintenance forever. */
const updaterVersion = (process.env.UPDATER_VERSION ?? "").trim() || null;
const updaterSelfUpdateCapable = true;
const updaterSelfUpdateEnabled = (process.env.AUTO_UPDATE_UPDATER ?? "true") === "true";
const testMode = process.env.SNAPDOG_UPDATER_TEST === "true";
function trustedStateFile(candidate, fallback) {
  const path = resolve(candidate ?? fallback);
  if (!testMode && !path.startsWith("/state/")) {
    throw new Error(`updater state files must remain below /state: ${path}`);
  }
  return path;
}
const swapResultFile = trustedStateFile(process.env.SWAP_RESULT_FILE, "/state/updater-swap.json");
const progressFile = trustedStateFile(process.env.PROGRESS_FILE, "/state/updater-progress.json");
function readStateJson(path) {
  // The path has already passed trustedStateFile(), which confines production
  // reads to /state. Tests intentionally use an isolated mkdtemp directory.
  return JSON.parse(readFileSync(path, "utf8"));
}
function writeStateJson(path, value) {
  const temporary = `${path}.tmp`;
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  writeFileSync(temporary, `${JSON.stringify(value)}\n`, { mode: 0o600 });
  renameSync(temporary, path);
}
/* Outcome of the last self-update, written by the detached helper that performed
 * it. The container that did the swap is gone by the time anyone can ask, so the
 * NEW updater reads the file and reports it — otherwise a failed or rolled-back
 * swap would be invisible outside the container logs. */
function lastSwap() {
  try {
    const value = readStateJson(swapResultFile);
    if (!value || typeof value.outcome !== "string") return null;
    if (!["succeeded", "failed", "rolled-back"].includes(value.outcome)) return null;
    return {
      outcome: value.outcome,
      detail: typeof value.detail === "string" ? value.detail.slice(0, 300) : null,
      at: typeof value.at === "string" ? value.at : null,
    };
  } catch {
    return null;
  }
}
const target = process.env.TARGET_CONTAINER ?? "snapdog";
const configFile = trustedStateFile(process.env.UPDATER_CONFIG_FILE, "/state/config.json");
const defaultConfig = {
  mode: (process.env.AUTO_APPLY ?? "true") === "true" ? "automatic" : "manual",
  maintenanceTime: process.env.MAINTENANCE_TIME ?? "02:00",
  timezone: process.env.TZ ?? "UTC",
  lastAutomaticAttemptDate: null,
};

if (!testMode) {
  if (token.length < 32) throw new Error("UPDATER_TOKEN must contain at least 32 characters");
  // snapdog.env.example ships UPDATER_TOKEN=replace-with-openssl-rand-hex-32, which
  // is exactly 32 characters and would otherwise pass the check above — leaving
  // this root-equivalent control API guarded by a token published in the repo.
  if (/replace[-_]with|change[-_]?me/i.test(token)) {
    throw new Error(
      "UPDATER_TOKEN still contains the example placeholder — generate a real value with: openssl rand -hex 32"
    );
  }
  if (!Number.isInteger(intervalSeconds) || intervalSeconds < 300)
    throw new Error("POLL_INTERVAL_SECONDS must be at least 300");
  if (!Number.isInteger(operationTimeoutMs) || operationTimeoutMs < 60_000)
    throw new Error("UPDATE_TIMEOUT_SECONDS must be at least 60");
  if (!Number.isInteger(port) || port < 1 || port > 65_535)
    throw new Error("CONTROL_PORT is invalid");
  if (!validateConfig(defaultConfig)) throw new Error("invalid default update schedule");
}

function loadConfig() {
  try {
    const stored = readStateJson(configFile);
    if (validateConfig(stored)) return { ...defaultConfig, ...stored };
    console.error("snapdog-updater: ignoring invalid persisted configuration");
  } catch (error) {
    if (error?.code !== "ENOENT")
      console.error(`snapdog-updater: cannot read persisted configuration: ${error}`);
  }
  return { ...defaultConfig };
}
let config = loadConfig();
function saveConfig() {
  writeStateJson(configFile, config);
}

const status = {
  state: "starting",
  currentVersion: null,
  availableVersion: null,
  updateAvailable: false,
  lastCheckedAt: null,
  lastCheckAttemptAt: null,
  lastUpdatedAt: null,
  lastError: null,
  releaseCheckStatus: "ok",
  releaseCheckError: null,
  releaseCheckRetryAt: null,
  updaterVersion,
  updaterUpdateAvailable: false,
  updaterSelfUpdateCapable,
  updaterSelfUpdateEnabled,
};
let active = false;

const PHASES = [
  "verifying",
  "backing-up",
  "deploying",
  "waiting-for-health",
  "done",
  "rolling-back",
  "failed",
];
/* The phase journal written by update.sh. The server is the thing being replaced,
 * so it cannot report its own restart — this is the only progress the admin UI
 * can show while the container is gone, and it is read back afterwards to
 * reconstruct what happened. */
function lastProgress() {
  try {
    const value = readStateJson(progressFile);
    if (!value || !PHASES.includes(value.phase)) return null;
    return {
      phase: value.phase,
      detail: typeof value.detail === "string" ? value.detail.slice(0, 200) : null,
      at: typeof value.at === "string" ? value.at : null,
      startedAt: typeof value.startedAt === "string" ? value.startedAt : null,
      failedPhase: PHASES.includes(value.failedPhase) ? value.failedPhase : null,
      rollbackAttempted: value.rollbackAttempted === true,
    };
  } catch {
    return null;
  }
}

/** Publish the new operation before /v1/apply answers. Without this synchronous
 * hand-off, the first UI poll can still read the terminal journal from the
 * previous update while the shell worker is only starting. */
function startProgressJournal(targetVersion, now = new Date()) {
  const at = now.toISOString();
  const temporary = `${progressFile}.tmp`;
  try {
    writeStateJson(progressFile, {
      phase: "verifying",
      detail: targetVersion || null,
      at,
      startedAt: at,
      failedPhase: null,
      rollbackAttempted: false,
    });
    return true;
  } catch (error) {
    try {
      rmSync(temporary, { force: true });
    } catch {
      // Journal cleanup is advisory too; never block the actual update.
    }
    console.error(`snapdog-updater: cannot reset progress journal: ${error}`);
    return false;
  }
}

function publicStatus() {
  return {
    ...status,
    updateMode: config.mode,
    maintenanceTime: config.maintenanceTime,
    timezone: config.timezone,
    updaterSwap: lastSwap(),
    progress: lastProgress(),
  };
}
function equalToken(candidate) {
  const a = Buffer.from(candidate ?? "");
  const b = Buffer.from(token);
  return a.length === b.length && timingSafeEqual(a, b);
}
function command(commandName, args, options = {}) {
  return new Promise((resolve, reject) => {
    const { quietStderr = false, ...spawnOptions } = options;
    const child = spawn(commandName, args, {
      timeout: operationTimeoutMs,
      killSignal: "SIGTERM",
      ...spawnOptions,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (data) => {
      stdout += data;
    });
    child.stderr.on("data", (data) => {
      stderr += data;
      if (!quietStderr) process.stderr.write(data);
    });
    child.on("error", reject);
    child.on("close", (code) =>
      code === 0
        ? resolve(stdout.trim())
        : reject(new Error(stderr.trim() || `${commandName} exited ${code}`))
    );
  });
}
async function currentVersion() {
  const image = await command("docker", ["inspect", "--format", "{{.Image}}", target]);
  return command("docker", [
    "image",
    "inspect",
    "--format",
    '{{ index .Config.Labels "org.opencontainers.image.version" }}',
    image,
  ]);
}
async function signedImageReady(repository, tag, identity) {
  try {
    await command(
      "cosign",
      [
        "verify",
        "--certificate-oidc-issuer",
        cosignIssuer,
        "--certificate-identity-regexp",
        identity,
        `${repository}:${tag}`,
      ],
      { timeout: 20_000, quietStderr: true }
    );
    return true;
  } catch {
    return false;
  }
}
async function releaseArtifactsReady(tag) {
  const checks = [signedImageReady(serverImageRepository, tag, serverIdentity)];
  if (updaterSelfUpdateEnabled)
    checks.push(signedImageReady(updaterImageRepository, tag, updaterIdentity));
  return (await Promise.all(checks)).every(Boolean);
}
let readinessRetryTimer = null;
let readinessRetryAttempt = 0;
let releaseCheckRetryTimer = null;
let releaseCheckRetryAttempt = 0;
function clearReadinessRetry() {
  if (readinessRetryTimer) clearTimeout(readinessRetryTimer);
  readinessRetryTimer = null;
  readinessRetryAttempt = 0;
}
function scheduleReadinessRetry() {
  if (readinessRetryTimer) clearTimeout(readinessRetryTimer);
  const delaySeconds = Math.min(15 * 2 ** readinessRetryAttempt, 300);
  readinessRetryAttempt += 1;
  readinessRetryTimer = setTimeout(() => {
    readinessRetryTimer = null;
    void check();
  }, delaySeconds * 1000);
  readinessRetryTimer.unref();
}
function clearReleaseCheckRetry() {
  if (releaseCheckRetryTimer) clearTimeout(releaseCheckRetryTimer);
  releaseCheckRetryTimer = null;
  releaseCheckRetryAttempt = 0;
  status.releaseCheckRetryAt = null;
}
function scheduleReleaseCheckRetry(requestedRetryAt = null) {
  if (releaseCheckRetryTimer) clearTimeout(releaseCheckRetryTimer);
  const fallbackDelay = Math.min(30 * 2 ** releaseCheckRetryAttempt, 300) * 1_000;
  const requestedDelay = requestedRetryAt
    ? Math.max(new Date(requestedRetryAt).getTime() - Date.now(), 0)
    : 0;
  const delay = Math.max(fallbackDelay, requestedDelay);
  releaseCheckRetryAttempt += 1;
  status.releaseCheckRetryAt = new Date(Date.now() + delay).toISOString();
  releaseCheckRetryTimer = setTimeout(() => {
    releaseCheckRetryTimer = null;
    void check();
  }, delay);
  releaseCheckRetryTimer.unref();
}
async function check() {
  if (active) return;
  active = true;
  const previousState = status.state;
  status.state = "checking";
  status.lastCheckAttemptAt = new Date().toISOString();
  try {
    const [current, available] = await Promise.all([
      currentVersion(),
      fetchReleaseVersion(releaseApi),
    ]);
    status.lastError = null;
    status.currentVersion = current || null;
    status.availableVersion = available;
    status.lastCheckedAt = new Date().toISOString();
    status.releaseCheckStatus = "ok";
    status.releaseCheckError = null;
    clearReleaseCheckRetry();
    const releaseIsNewer = newer(current, available);
    const artifactsReady = !releaseIsNewer || (await releaseArtifactsReady(available));
    const decision = releaseDecision(current, available, artifactsReady);
    status.updateAvailable = decision.updateAvailable;
    /* Both images are pinned to the same release tag by the deployment assets, so
     * the server's candidate release is also the updater's candidate. */
    status.updaterUpdateAvailable = artifactsReady && newer(updaterVersion, available);
    const progress = lastProgress();
    /* A periodic release check must not erase the outcome of a failed deployment
     * while the old version is still running. Keep retry available, but preserve
     * the actionable failure and its real journal detail in the UI. */
    if (releaseIsNewer && ["failed", "rolling-back"].includes(progress?.phase)) {
      status.state = "failed";
      status.lastError = progress?.detail ?? "The previous update attempt failed";
    } else {
      status.state = decision.state;
    }
    if (status.state === "preparing") scheduleReadinessRetry();
    else clearReadinessRetry();
  } catch (error) {
    clearReadinessRetry();
    if (error instanceof ReleaseCheckError) {
      status.state = ["available", "preparing", "current", "failed"].includes(previousState)
        ? previousState
        : "current";
      status.releaseCheckStatus = "degraded";
      status.releaseCheckError = error.code;
      scheduleReleaseCheckRetry(error.retryAt);
      console.error(`snapdog-updater: release check degraded: ${error.message}`);
    } else {
      clearReleaseCheckRetry();
      status.state = "failed";
      status.lastError = String(error instanceof Error ? error.message : error).slice(0, 500);
    }
  } finally {
    active = false;
  }
  await tryScheduledApply();
}
async function apply() {
  if (active) return false;
  active = true;
  status.state = "updating";
  status.lastError = null;
  try {
    const targetVersion = status.availableVersion;
    startProgressJournal(targetVersion);
    await command("/usr/local/bin/snapdog-update", [], {
      // The control plane already resolved and validated this exact release.
      // Hand it to the deployment worker so an unrelated GitHub API outage
      // cannot break an update after the administrator has started it.
      env: { ...process.env, UPDATE_ONCE: "true", TARGET_VERSION: targetVersion },
    });
    const runningVersion = await currentVersion();
    if (!reached(runningVersion, targetVersion)) {
      throw new Error(
        `update command completed but ${runningVersion || "no version"} is running instead of ${targetVersion || "the requested release"}`
      );
    }
    status.currentVersion = runningVersion;
    status.updateAvailable = false;
    status.lastUpdatedAt = new Date().toISOString();
    status.state = "current";
  } catch (error) {
    const progress = lastProgress();
    status.state = "failed";
    status.lastError =
      progress?.detail ?? String(error instanceof Error ? error.message : error).slice(-500);
  } finally {
    active = false;
  }
  return true;
}
async function tryScheduledApply(now = new Date()) {
  if (active || !status.updateAvailable || config.mode !== "automatic") return false;
  const clock = zonedClock(now, config.timezone);
  if (clock.time !== config.maintenanceTime || clock.date === config.lastAutomaticAttemptDate)
    return false;
  config.lastAutomaticAttemptDate = clock.date;
  saveConfig();
  return apply();
}
function send(response, code, body) {
  response.writeHead(code, { "content-type": "application/json", "cache-control": "no-store" });
  response.end(JSON.stringify(body));
}
function readJson(request) {
  return new Promise((resolve, reject) => {
    let body = "";
    request.setEncoding("utf8");
    request.on("data", (chunk) => {
      body += chunk;
      if (body.length > 4096) request.destroy();
    });
    request.on("end", () => {
      try {
        resolve(JSON.parse(body));
      } catch {
        reject(new Error("invalid_json"));
      }
    });
    request.on("error", reject);
  });
}
function startServer() {
  return createServer(async (request, response) => {
    try {
      if (!equalToken(request.headers.authorization?.replace(/^Bearer /, "")))
        return send(response, 401, { error: "unauthorized" });
      if (request.method === "GET" && request.url === "/v1/status")
        return send(response, 200, publicStatus());
      if (request.method === "POST" && request.url === "/v1/check") {
        if (active)
          return send(response, 409, { error: "operation_in_progress", status: publicStatus() });
        void check();
        return send(response, 202, publicStatus());
      }
      if (request.method === "POST" && request.url === "/v1/apply") {
        if (active)
          return send(response, 409, { error: "operation_in_progress", status: publicStatus() });
        if (!status.updateAvailable) return send(response, 200, publicStatus());
        void apply();
        return send(response, 202, publicStatus());
      }
      if (request.method === "POST" && request.url === "/v1/config") {
        if (active)
          return send(response, 409, { error: "operation_in_progress", status: publicStatus() });
        const input = await readJson(request);
        if (!validateConfig(input)) return send(response, 400, { error: "invalid_config" });
        config = {
          mode: input.mode,
          maintenanceTime: input.maintenanceTime,
          timezone: input.timezone,
          lastAutomaticAttemptDate: null,
        };
        saveConfig();
        void tryScheduledApply();
        return send(response, 200, publicStatus());
      }
      return send(response, 404, { error: "not_found" });
    } catch (error) {
      console.error(`snapdog-updater: control request failed: ${error}`);
      if (!response.headersSent) return send(response, 400, { error: "invalid_request" });
    }
  }).listen(port, "0.0.0.0", () =>
    console.error(`snapdog-updater: control API listening on ${port}`)
  );
}

if (!testMode) {
  const server = startServer();
  const shutdown = () => {
    server.close(() => process.exit(0));
    setTimeout(() => process.exit(1), 10_000).unref();
  };
  process.once("SIGTERM", shutdown);
  process.once("SIGINT", shutdown);
  void check();
  setInterval(() => void check(), intervalSeconds * 1000).unref();
  setInterval(() => void tryScheduledApply(), 30_000).unref();
}

export {
  equalToken,
  newer,
  reached,
  releaseDecision,
  validateConfig,
  zonedClock,
  publicStatus,
  startProgressJournal,
};
