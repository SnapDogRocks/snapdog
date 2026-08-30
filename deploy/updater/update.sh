#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 Fabian Schmieder. All rights reserved.

set -Eeuo pipefail

readonly RELEASE_API="${RELEASE_API:-https://api.github.com/repos/SnapDogRocks/snapdog/releases/latest}"
readonly TARGET_VERSION="${TARGET_VERSION:-}"
readonly IMAGE_REPOSITORY="${IMAGE_REPOSITORY:-ghcr.io/snapdogrocks/snapdog}"
readonly COMPOSE_FILE="${COMPOSE_FILE:-/stack/docker-compose.yml}"
readonly SNAPDOG_ENV_FILE="${SNAPDOG_ENV_FILE:-/run/snapdog/snapdog.env}"
export COMPOSE_ENV_FILE="${COMPOSE_ENV_FILE:-$SNAPDOG_ENV_FILE}"
readonly COMPOSE_ENV_FILE
HOST_STACK_DIR="${HOST_STACK_DIR:-}"
readonly ENV_BACKUP_FILE="${ENV_BACKUP_FILE:-/state/snapdog.env.backup}"
readonly COMPOSE_PROJECT="${COMPOSE_PROJECT:-snapdog}"
readonly COMPOSE_SERVICE="${COMPOSE_SERVICE:-server}"
readonly TARGET_CONTAINER="${TARGET_CONTAINER:-snapdog}"
readonly POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-900}"
readonly READINESS_URL="${READINESS_URL:-http://server:3000/api/v1/health}"
readonly READINESS_TIMEOUT_SECONDS="${READINESS_TIMEOUT_SECONDS:-120}"
readonly COSIGN_ISSUER="https://token.actions.githubusercontent.com"
readonly COSIGN_IDENTITY_REGEXP='^https://github\.com/SnapDogRocks/snapdog/\.github/workflows/release\.yml@refs/tags/v[0-9]+\.[0-9]+\.[0-9]+$'
# The updater image is built by a DIFFERENT workflow than the server image, so it
# needs its own Sigstore identity — reusing the server regexp would reject every
# legitimate updater image (or, worse, accept the wrong provenance).
readonly UPDATER_IMAGE_REPOSITORY="${UPDATER_IMAGE_REPOSITORY:-ghcr.io/snapdogrocks/snapdog-updater}"
readonly COSIGN_UPDATER_IDENTITY_REGEXP='^https://github\.com/SnapDogRocks/snapdog/\.github/workflows/release\.yml@refs/tags/v[0-9]+\.[0-9]+\.[0-9]+$'
readonly UPDATER_SERVICE="${UPDATER_SERVICE:-updater}"
# A detached helper replaces this container, verifies health, and rolls back a
# failed candidate. Keep this enabled by default for unattended installations.
readonly AUTO_UPDATE_UPDATER="${AUTO_UPDATE_UPDATER:-true}"
readonly UPDATER_SWAP_TIMEOUT_SECONDS="${UPDATER_SWAP_TIMEOUT_SECONDS:-180}"
# The swap outcome outlives the container that performed it, so the NEW updater
# can report what happened to the admin UI.
readonly SWAP_RESULT_FILE="${SWAP_RESULT_FILE:-/state/updater-swap.json}"
# Phase journal. The server cannot narrate its own restart — it is the thing being
# replaced — so the updater records each step here and the admin UI reads it back
# through /v1/status. Without this the operator only ever saw a state badge.
readonly PROGRESS_FILE="${PROGRESS_FILE:-/state/updater-progress.json}"
CURRENT_PHASE=""
FAILED_PHASE=""
ROLLBACK_ATTEMPTED=false

log() {
  printf '%s snapdog-updater: %s\n' "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" "$*" >&2
}

die() {
  log "ERROR: $*"
  return 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

validate_config() {
  for command in curl jq docker cosign; do
    require_command "$command"
  done
  # `-r` accepts directories. Docker creates a directory when a bind source is
  # missing, which previously let a poisoned /stack mount pass preflight and
  # fail only after a backup had already been taken.
  [[ -f "$COMPOSE_FILE" && -r "$COMPOSE_FILE" ]] ||
    die "compose file is not a readable regular file: $COMPOSE_FILE"
  [[ -f "$SNAPDOG_ENV_FILE" && -r "$SNAPDOG_ENV_FILE" ]] ||
    die "environment file is not a readable regular file: $SNAPDOG_ENV_FILE"
  [[ -w "$SNAPDOG_ENV_FILE" ]] || die "environment file is not writable: $SNAPDOG_ENV_FILE"
  HOST_STACK_DIR="$(resolve_host_stack_dir)" || return 1
  [[ "$POLL_INTERVAL_SECONDS" =~ ^[0-9]+$ ]] || die "POLL_INTERVAL_SECONDS must be numeric"
  (( POLL_INTERVAL_SECONDS >= 300 )) || die "POLL_INTERVAL_SECONDS must be at least 300"
  [[ "$READINESS_TIMEOUT_SECONDS" =~ ^[0-9]+$ ]] || die "READINESS_TIMEOUT_SECONDS must be numeric"
  (( READINESS_TIMEOUT_SECONDS >= 30 )) || die "READINESS_TIMEOUT_SECONDS must be at least 30"
}

# Compose resolves relative bind sources on the Docker daemon host, not inside
# this container. New stacks capture the host working directory explicitly.
# Older, still-healthy stacks can derive it from the compose-file bind mount.
resolve_host_stack_dir() {
  local source
  if [[ -n "$HOST_STACK_DIR" ]]; then
    [[ "$HOST_STACK_DIR" == /* ]] || {
      die "HOST_STACK_DIR must be an absolute host path"
      return 1
    }
    printf '%s\n' "$HOST_STACK_DIR"
    return 0
  fi

  source="$(docker inspect --format '{{range .Mounts}}{{if eq .Destination "'"$COMPOSE_FILE"'"}}{{.Source}}{{end}}{{end}}' \
    "${HOSTNAME:-}" 2>/dev/null || true)"
  [[ -n "$source" && "$source" == /* ]] || {
    die "could not determine the host stack directory"
    return 1
  }
  dirname "$source"
}

compose_stack() {
  local host_stack_dir="$HOST_STACK_DIR"
  [[ -n "$host_stack_dir" && "$host_stack_dir" == /* ]] || {
    die "host stack directory is unavailable"
    return 1
  }
  HOST_STACK_DIR="$host_stack_dir" docker compose \
    --env-file "$SNAPDOG_ENV_FILE" \
    --project-directory "$host_stack_dir" \
    --project-name "$COMPOSE_PROJECT" \
    --file "$COMPOSE_FILE" "$@"
}

github_curl() {
  local -a headers=(
    -H "Accept: application/vnd.github+json"
    -H "X-GitHub-Api-Version: 2022-11-28"
    -H "User-Agent: snapdog-updater"
  )
  if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    headers+=(-H "Authorization: Bearer ${GITHUB_TOKEN}")
  fi
  curl --fail --silent --show-error --location --max-time 30 \
    --retry 3 --retry-all-errors --retry-delay 1 --retry-max-time 25 \
    "${headers[@]}" "$@"
}

latest_server_tag() {
  local releases tag
  releases="$(github_curl "$RELEASE_API")" || return 1
  tag="$(jq -er '
    (if type == "array" then . else [.] end)
    | map(select(
        .draft == false
        and .prerelease == false
        and (.tag_name | type == "string" and test("^v[0-9]+\\.[0-9]+\\.[0-9]+$"))
      ))
    | sort_by(.tag_name | ltrimstr("v") | split(".") | map(tonumber))
    | last
    | .tag_name
  ' <<<"$releases")" || {
    die "GitHub Releases contains no stable SnapDog server release"
    return 1
  }
  printf '%s\n' "$tag"
}

current_version() {
  local image_id
  image_id="$(docker inspect --format '{{.Image}}' "$TARGET_CONTAINER" 2>/dev/null)" || return 1
  docker image inspect --format '{{ index .Config.Labels "org.opencontainers.image.version" }}' \
    "$image_id" 2>/dev/null
}

version_is_newer() {
  local current="${1#v}" candidate="${2#v}"
  [[ "$current" != "$candidate" ]] || return 1
  [[ "$(printf '%s\n%s\n' "$current" "$candidate" | sort -V | tail -n1)" == "$candidate" ]]
}

verify_image() {
  local candidate="$1" repository="${2:-$IMAGE_REPOSITORY}" identity="${3:-$COSIGN_IDENTITY_REGEXP}" digest
  if ! docker pull --quiet "$candidate" >/dev/null; then
    die "signed release image is not available yet: $candidate"
    return 1
  fi
  if ! digest="$(docker image inspect --format '{{ json .RepoDigests }}' "$candidate" |
      jq -er --arg prefix "${repository}@sha256:" 'map(select(startswith($prefix))) | first')"; then
    die "pulled image has no trusted digest for $repository"
    return 1
  fi
  [[ "$digest" == "${repository}@sha256:"* ]] ||
    { die "pulled image has unexpected digest identity: $digest"; return 1; }

  log "verifying Sigstore identity for $digest"
  if ! cosign verify \
    --certificate-oidc-issuer "$COSIGN_ISSUER" \
    --certificate-identity-regexp "$identity" \
    "$digest" >/dev/null; then
    die "image signature or publishing identity is invalid: $candidate"
    return 1
  fi
}

compose_deploy() {
  local image="$1"
  SNAPDOG_IMAGE="$image" compose_stack \
    up --detach --no-deps --pull never --force-recreate "$COMPOSE_SERVICE"
}

# Rewrite exactly one KEY=image line, keeping a restorable copy first. Shared by
# the server pin and the updater pin so the careful backup/verify path is not
# duplicated — the updater pin is what stops a later `docker compose up` from
# silently reinstating the old updater.
persist_env_pin() {
  local key="$1" image="$2" count temporary backup_temporary
  count="$(grep -c "^${key}=" "$SNAPDOG_ENV_FILE" || true)"
  [[ "$count" == "1" ]] || {
    die "environment file must contain exactly one ${key} setting"
    return 1
  }

  mkdir -p "$(dirname "$ENV_BACKUP_FILE")"
  temporary="$(mktemp)"
  backup_temporary="${ENV_BACKUP_FILE}.tmp"
  awk -v key="$key" -v image="$image" '
    index($0, key "=") == 1 { print key "=" image; next }
    { print }
  ' "$SNAPDOG_ENV_FILE" >"$temporary"
  cp -p "$SNAPDOG_ENV_FILE" "$backup_temporary"
  mv -f "$backup_temporary" "$ENV_BACKUP_FILE"

  if ! cp "$temporary" "$SNAPDOG_ENV_FILE" ||
      ! grep -Fx "${key}=${image}" "$SNAPDOG_ENV_FILE" >/dev/null; then
    cp "$ENV_BACKUP_FILE" "$SNAPDOG_ENV_FILE" || true
    rm -f "$temporary"
    die "could not persist ${key}"
    return 1
  fi
  rm -f "$temporary"
  sync -f "$SNAPDOG_ENV_FILE" 2>/dev/null || true
  log "persisted image pin: ${key}=${image}"
}

persist_server_image() {
  persist_env_pin SNAPDOG_IMAGE "$1"
}

wait_for_readiness() {
  local publish_phase="${1:-true}"
  local deadline=$((SECONDS + READINESS_TIMEOUT_SECONDS))
  [[ "$publish_phase" == "true" ]] && set_phase "waiting-for-health"
  while (( SECONDS < deadline )); do
    if [[ "$(curl --fail --silent --show-error --max-time 5 "$READINESS_URL")" == "ready" ]]; then
      return 0
    fi
    sleep 3
  done
  return 1
}

deploy_with_rollback() {
  local candidate="$1" old_id rollback_ref
  old_id="$(docker inspect --format '{{.Image}}' "$TARGET_CONTAINER")"
  rollback_ref="${IMAGE_REPOSITORY}:rollback"
  docker image tag "$old_id" "$rollback_ref"

  set_phase "deploying" "$candidate"
  log "deploying $candidate"
  if compose_deploy "$candidate" && wait_for_readiness true; then
    if persist_server_image "$candidate"; then
      log "deployment healthy: $candidate"
      set_phase "done" "$candidate"
      return 0
    fi
    log "deployment healthy but image pin persistence failed; rolling back"
  fi

  log "deployment failed; rolling back to $old_id"
  set_phase "rolling-back" "$rollback_ref"
  if ! compose_deploy "$rollback_ref" || ! wait_for_readiness false; then
    set_phase "failed" "automatic rollback failed; operator intervention required" "rolling-back"
    die "automatic rollback failed; operator intervention required"
  fi
  set_phase "failed" "rolled back because the new release did not become ready"
  die "deployment rolled back because the new release did not become ready"
}

# ── Updater self-update ───────────────────────────────────────────────
#
# The updater must never recreate its own container directly: `compose up` would
# kill the very process issuing the call, leaving the swap half-done with nothing
# left to finish or undo it. Instead the running (known-good) updater verifies the
# candidate, then hands the swap to a DETACHED one-shot container and exits. The
# one-shot is not a child process, so it survives its parent being replaced.
#
# It runs from the CURRENT image on purpose: if the new image were broken, a
# helper built from it could not roll anything back.

updater_container_id() {
  # Resolved through compose rather than a fixed name: deployments created before
  # `container_name` was added to the compose file run as <project>-updater-1.
  compose_stack ps --quiet "$UPDATER_SERVICE" 2>/dev/null | head -n1
}

# Advisory like record_swap_result: a failed journal write must never abort an
# update. `phase` is the step now running, `outcome` closes the journal out.
set_phase() {
  local phase="$1" detail="${2:-}" explicit_failed_phase="${3:-}" temporary="${PROGRESS_FILE}.tmp"
  if [[ "$phase" == "verifying" ]]; then
    FAILED_PHASE=""
    ROLLBACK_ATTEMPTED=false
  elif [[ "$phase" == "rolling-back" ]]; then
    [[ -n "$FAILED_PHASE" ]] || FAILED_PHASE="$CURRENT_PHASE"
    ROLLBACK_ATTEMPTED=true
  elif [[ "$phase" == "failed" ]]; then
    if [[ -n "$explicit_failed_phase" ]]; then
      FAILED_PHASE="$explicit_failed_phase"
    elif [[ -z "$FAILED_PHASE" ]]; then
      FAILED_PHASE="$CURRENT_PHASE"
    fi
  fi
  CURRENT_PHASE="$phase"
  log "phase: $phase${detail:+ ($detail)}"
  mkdir -p "$(dirname "$PROGRESS_FILE")" 2>/dev/null || true
  if jq -n --arg phase "$phase" --arg detail "$detail" \
           --arg at "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" \
           --arg started "${PHASE_STARTED_AT:-}" \
           --arg failedPhase "$FAILED_PHASE" \
           --argjson rollbackAttempted "$ROLLBACK_ATTEMPTED" \
           '{phase: $phase, detail: (if $detail == "" then null else $detail end),
             at: $at, startedAt: (if $started == "" then $at else $started end),
             failedPhase: (if $failedPhase == "" then null else $failedPhase end),
             rollbackAttempted: $rollbackAttempted}' \
       >"$temporary" 2>/dev/null; then
    mv -f "$temporary" "$PROGRESS_FILE" 2>/dev/null || rm -f "$temporary" 2>/dev/null || true
  else
    rm -f "$temporary" 2>/dev/null || true
  fi
}

# Recording the outcome must never abort a swap, so every step here is advisory.
record_swap_result() {
  local outcome="$1" detail="$2" temporary="${SWAP_RESULT_FILE}.tmp"
  mkdir -p "$(dirname "$SWAP_RESULT_FILE")" 2>/dev/null || true
  if jq -n --arg outcome "$outcome" --arg detail "$detail" \
           --arg at "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" \
           '{outcome: $outcome, detail: $detail, at: $at}' \
       >"$temporary" 2>/dev/null; then
    mv -f "$temporary" "$SWAP_RESULT_FILE" 2>/dev/null || rm -f "$temporary" 2>/dev/null || true
  else
    rm -f "$temporary" 2>/dev/null || true
  fi
}

# A terminal self-update result describes one attempt only. Clearing it is
# advisory for the same reason as writing it: update safety must not depend on
# the journal volume being writable.
clear_swap_result() {
  rm -f "$SWAP_RESULT_FILE" 2>/dev/null || true
}

wait_for_container_health() {
  local container="$1" deadline=$((SECONDS + UPDATER_SWAP_TIMEOUT_SECONDS)) state health
  while (( SECONDS < deadline )); do
    state="$(docker inspect --format '{{.State.Status}}' "$container" 2>/dev/null || true)"
    health="$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "$container" 2>/dev/null || true)"
    if [[ "$state" == "running" && ( "$health" == "healthy" || "$health" == "none" ) ]]; then
      return 0
    fi
    [[ "$state" == "exited" || "$state" == "dead" ]] && return 1
    sleep 3
  done
  return 1
}

compose_up_updater() {
  local image="$1"
  UPDATER_IMAGE="$image" compose_stack \
    up --detach --no-deps --pull never --force-recreate "$UPDATER_SERVICE"
}

# Runs INSIDE the one-shot helper. Owns the rollback: if the new updater does not
# come up healthy, nothing else is left that could undo the swap.
swap_updater() {
  local candidate="$1" previous_image previous_id new_id
  previous_id="$(updater_container_id)"
  previous_image="$(docker inspect --format '{{.Config.Image}}' "$previous_id" 2>/dev/null || true)"
  [[ -n "$previous_image" ]] || {
    record_swap_result failed "could not determine the running updater image"
    die "could not determine the running updater image"
    return 1
  }

  log "swapping updater: $previous_image -> $candidate"
  if compose_up_updater "$candidate"; then
    new_id="$(updater_container_id)"
    if [[ -n "$new_id" ]] && wait_for_container_health "$new_id"; then
      if persist_env_pin UPDATER_IMAGE "$candidate"; then
        log "updater swap healthy: $candidate"
        record_swap_result succeeded "$candidate"
        return 0
      fi
      log "updater came up healthy but the pin could not be persisted; rolling back"
    else
      log "replacement updater did not become healthy; rolling back"
    fi
  else
    log "could not start the replacement updater; rolling back"
  fi

  if compose_up_updater "$previous_image" && wait_for_container_health "$(updater_container_id)"; then
    record_swap_result "rolled-back" "restored $previous_image"
    die "updater swap rolled back to $previous_image"
    return 1
  fi
  record_swap_result failed "rollback to $previous_image failed; operator intervention required"
  die "updater rollback failed; operator intervention required"
  return 1
}

# Runs in the ORIGINAL updater, after a healthy server update.
schedule_updater_swap() {
  local tag="$1" candidate self_id self_image
  [[ "$AUTO_UPDATE_UPDATER" == "true" ]] || return 0

  candidate="${UPDATER_IMAGE_REPOSITORY}:${tag}"
  if [[ "${UPDATER_VERSION:-}" == "$tag" ]]; then
    return 0
  fi
  if [[ "${UPDATER_VERSION:-}" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+$ ]] &&
      ! version_is_newer "$UPDATER_VERSION" "$tag"; then
    return 0
  fi

  # A previous terminal outcome belongs to the previous attempt. Leaving it in
  # place makes the UI fail a new, still-running update before the detached
  # helper has had a chance to publish its own result.
  clear_swap_result

  # Verify here, in the known-good container, so a helper is never launched for an
  # image that failed its signature check.
  if ! verify_image "$candidate" "$UPDATER_IMAGE_REPOSITORY" "$COSIGN_UPDATER_IDENTITY_REGEXP"; then
    log "updater image signature verification failed; keeping ${UPDATER_VERSION:-current}"
    record_swap_result failed "signature verification failed for $candidate"
    return 1
  fi

  self_id="$(updater_container_id)"
  self_image="$(docker inspect --format '{{.Config.Image}}' "$self_id" 2>/dev/null || true)"
  if [[ -z "$self_id" || -z "$self_image" ]]; then
    log "could not resolve this updater container; skipping self-update"
    record_swap_result failed "could not resolve the running updater container"
    return 1
  fi

  log "handing the updater swap to a detached helper (${UPDATER_VERSION:-unknown} -> $tag)"
  # --volumes-from inherits the socket and the stack mounts. Passing -v here would
  # be wrong: paths inside this container are not paths on the host.
  if ! docker run --detach --rm \
    --volumes-from "$self_id" \
    --network none \
    --label snapdog.role=updater-swap \
    --env "COMPOSE_PROJECT=$COMPOSE_PROJECT" \
    --env "COMPOSE_FILE=$COMPOSE_FILE" \
    --env "SNAPDOG_ENV_FILE=$SNAPDOG_ENV_FILE" \
    --env "HOST_STACK_DIR=$HOST_STACK_DIR" \
    --env "COMPOSE_ENV_FILE=$COMPOSE_ENV_FILE" \
    --env "UPDATER_SERVICE=$UPDATER_SERVICE" \
    --env "UPDATER_SWAP_TIMEOUT_SECONDS=$UPDATER_SWAP_TIMEOUT_SECONDS" \
    --env "SWAP_RESULT_FILE=$SWAP_RESULT_FILE" \
    --entrypoint /usr/local/bin/snapdog-update \
    "$self_image" --swap-updater "$candidate" >/dev/null; then
    record_swap_result failed "could not start the detached updater swap helper"
    return 1
  fi
}

update_once() {
  local tag candidate current
  if [[ -n "$TARGET_VERSION" ]]; then
    if [[ ! "$TARGET_VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
      die "target server version is invalid"
      return 1
    fi
    tag="$TARGET_VERSION"
  else
    if ! tag="$(latest_server_tag)"; then
      die "could not determine latest server release"
      return 1
    fi
  fi
  candidate="${IMAGE_REPOSITORY}:${tag}"
  current="$(current_version 2>/dev/null || true)"

  if [[ "$current" == "$tag" || "$current" == "${tag#v}" ]]; then
    log "already current: $tag"
    return 0
  fi
  if [[ "$current" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+$ ]] && ! version_is_newer "$current" "$tag"; then
    log "refusing downgrade from $current to $tag"
    return 0
  fi

  log "new server release detected: ${current:-unknown} -> $tag"
  # The update overlay starts polling before the server deployment completes.
  # Do not let a terminal result from an older updater swap fail this new run.
  clear_swap_result
  PHASE_STARTED_AT="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
  export PHASE_STARTED_AT
  set_phase "verifying" "$tag"
  if ! verify_image "$candidate"; then
    set_phase "failed" "release image is unavailable or did not pass signature verification"
    return 1
  fi
  deploy_with_rollback "$candidate" || return 1
  # Only after the server is verifiably healthy — never leave the stack mid-update
  # with a swapped updater on top.
  schedule_updater_swap "$tag" || log "updater self-update skipped"
}

main() {
  if [[ "${1:-}" == "--swap-updater" ]]; then
    require_command docker
    require_command jq
    [[ -n "$HOST_STACK_DIR" && "$HOST_STACK_DIR" == /* ]] ||
      { die "--swap-updater needs an absolute HOST_STACK_DIR"; return 1; }
    [[ -n "${2:-}" ]] || die "--swap-updater needs an image reference"
    swap_updater "$2"
    return
  fi
  validate_config
  if [[ "${UPDATE_ONCE:-false}" == "true" ]]; then
    update_once
    return
  fi

  log "watching stable server releases every ${POLL_INTERVAL_SECONDS}s"
  while true; do
    update_once || log "cycle failed; keeping the current deployment"
    sleep "$POLL_INTERVAL_SECONDS" &
    wait $!
  done
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
