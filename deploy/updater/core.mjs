// SPDX-License-Identifier: GPL-3.0-only

const RELEASE_CHECK_ATTEMPTS = 3;
const FIRST_RELEASE_CHECK_DELAY_MS = 500;
const LATER_RELEASE_CHECK_DELAY_MS = 1_500;
const INLINE_RATE_LIMIT_WAIT_MS = 5_000;

export class ReleaseCheckError extends Error {
  constructor(code, message, retryAt = null) {
    super(message);
    this.name = "ReleaseCheckError";
    this.code = code;
    this.retryAt = retryAt;
  }
}

function validTimezone(value) {
  try {
    new Intl.DateTimeFormat("en", { timeZone: value }).format();
    return true;
  } catch {
    return false;
  }
}

export function validateConfig(value) {
  return (
    value &&
    ["manual", "automatic"].includes(value.mode) &&
    /^([01]\d|2[0-3]):[0-5]\d$/.test(value.maintenanceTime) &&
    typeof value.timezone === "string" &&
    value.timezone.length <= 100 &&
    validTimezone(value.timezone) &&
    (value.lastAutomaticAttemptDate == null ||
      /^\d{4}-\d{2}-\d{2}$/.test(value.lastAutomaticAttemptDate))
  );
}

function versionParts(value) {
  const match = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(value ?? "");
  if (!match) return null;
  return { major: Number(match[1]), minor: Number(match[2]), patch: Number(match[3]) };
}

function compareVersions(left, right) {
  const a = versionParts(left);
  const b = versionParts(right);
  if (!a || !b) return null;
  if (a.major !== b.major) return a.major - b.major;
  if (a.minor !== b.minor) return a.minor - b.minor;
  return a.patch - b.patch;
}

export function newer(current, candidate) {
  if (!versionParts(candidate)) return false;
  if (!versionParts(current)) return true;
  return compareVersions(candidate, current) > 0;
}

export function reached(current, targetVersion) {
  const comparison = compareVersions(current, targetVersion);
  return comparison != null && comparison >= 0;
}

export function releaseDecision(current, available, artifactsReady) {
  const releaseIsNewer = newer(current, available);
  return {
    state:
      releaseIsNewer && !artifactsReady ? "preparing" : releaseIsNewer ? "available" : "current",
    updateAvailable: releaseIsNewer && artifactsReady,
  };
}

export function stableServerReleaseTag(payload) {
  const releases = Array.isArray(payload) ? payload : [payload];
  return (
    releases
      .filter(
        (release) =>
          release &&
          release.draft === false &&
          release.prerelease === false &&
          /^v\d+\.\d+\.\d+$/.test(release.tag_name ?? "")
      )
      .map((release) => release.tag_name)
      .sort((left, right) => compareVersions(right, left) ?? 0)[0] ?? null
  );
}

export function zonedClock(date, timezone) {
  const values = Object.fromEntries(
    new Intl.DateTimeFormat("en-CA", {
      timeZone: timezone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    })
      .formatToParts(date)
      .filter((part) => part.type !== "literal")
      .map((part) => [part.type, part.value])
  );
  return {
    date: `${values.year}-${values.month}-${values.day}`,
    time: `${values.hour}:${values.minute}`,
  };
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

export function retryAtFromHeaders(headers, now = Date.now()) {
  const retryAfter = Number(headers.get("retry-after"));
  if (Number.isFinite(retryAfter) && retryAfter >= 0)
    return new Date(now + retryAfter * 1_000).toISOString();
  const reset = Number(headers.get("x-ratelimit-reset"));
  if (Number.isFinite(reset) && reset > 0) return new Date(reset * 1_000).toISOString();
  return null;
}

function normalizeReleaseCheckError(error) {
  if (error instanceof ReleaseCheckError) return error;
  const timeout = error?.name === "TimeoutError" || error?.name === "AbortError";
  return new ReleaseCheckError(
    timeout ? "upstream-timeout" : "network-error",
    timeout ? "GitHub Releases request timed out" : "GitHub Releases request failed"
  );
}

export function fetchReleaseVersion(
  api,
  fetchImpl = globalThis.fetch,
  sleepImpl = sleep,
  now = () => Date.now(),
  githubToken = process.env.GITHUB_TOKEN ?? ""
) {
  const headers = { Accept: "application/vnd.github+json", "User-Agent": "snapdog-updater" };
  if (githubToken) headers.Authorization = `Bearer ${githubToken}`;
  let lastError = null;
  function attemptReleaseCheck(attempt) {
    let retryDelay =
      attempt === 0 ? FIRST_RELEASE_CHECK_DELAY_MS : LATER_RELEASE_CHECK_DELAY_MS;
    return fetchImpl(api, {
      headers,
      signal: AbortSignal.timeout(30_000),
    })
      .then((response) => {
        if (response.ok) {
          return response
            .json()
            .catch(() => null)
            .then((payload) => {
              const tag = stableServerReleaseTag(payload);
              if (tag) return { done: true, tag };
              lastError = new ReleaseCheckError(
                "invalid-response",
                "GitHub Releases returned no stable SnapDog server release"
              );
              return { done: false };
            });
        } else if (
          response.status === 429 ||
          (response.status === 403 &&
            (response.headers.get("x-ratelimit-remaining") === "0" ||
              response.headers.has("retry-after")))
        ) {
          const retryAt = retryAtFromHeaders(response.headers, now());
          lastError = new ReleaseCheckError(
            "rate-limited",
            `GitHub Releases rate limit returned HTTP ${response.status}`,
            retryAt
          );
          const wait = retryAt ? new Date(retryAt).getTime() - now() : Infinity;
          if (wait > INLINE_RATE_LIMIT_WAIT_MS) throw lastError;
          retryDelay = Math.max(retryDelay, wait);
        } else if (
          response.status === 408 ||
          response.status === 425 ||
          response.status >= 500
        ) {
          lastError = new ReleaseCheckError(
            "upstream-unavailable",
            `GitHub Releases temporarily returned HTTP ${response.status}`
          );
        } else {
          throw new ReleaseCheckError(
            "request-rejected",
            `GitHub Releases returned HTTP ${response.status}`
          );
        }
        return { done: false };
      })
      .catch((error) => {
        lastError = normalizeReleaseCheckError(error);
        if (["rate-limited", "request-rejected"].includes(lastError.code)) throw lastError;
        return { done: false };
      })
      .then((result) => {
        if (result.done) return result.tag;
        if (attempt < RELEASE_CHECK_ATTEMPTS - 1) {
          return Promise.resolve(sleepImpl(retryDelay)).then(() => attemptReleaseCheck(attempt + 1));
        }
        throw lastError ?? new ReleaseCheckError("network-error", "GitHub Releases request failed");
      });
  }
  return attemptReleaseCheck(0);
}
