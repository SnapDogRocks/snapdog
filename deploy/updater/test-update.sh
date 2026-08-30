#!/usr/bin/env bash
set -Eeuo pipefail

export UPDATE_ONCE=true
export AUTO_UPDATE_UPDATER=false
export COMPOSE_FILE="${BASH_SOURCE[0]}"
test_directory="$(mktemp -d)"
trap 'rm -rf "$test_directory"' EXIT
export SNAPDOG_ENV_FILE="${test_directory}/.env"
export COMPOSE_ENV_FILE="$SNAPDOG_ENV_FILE"
export ENV_BACKUP_FILE="${test_directory}/state/snapdog.env.backup"
export PROGRESS_FILE="${test_directory}/state/progress.json"
printf '%s\n' \
  'SNAPDOG_IMAGE=ghcr.io/snapdogrocks/snapdog:v1.8.1' \
  'UPDATER_IMAGE=ghcr.io/snapdogrocks/snapdog-updater:v1.8.1' \
  'SOME_SETTING=preserved' >"$SNAPDOG_ENV_FILE"
# shellcheck disable=SC1091
source "$(dirname "${BASH_SOURCE[0]}")/update.sh"

assert() { "$@" || { printf 'FAILED: %q ' "$@" >&2; printf '\n' >&2; exit 1; }; }

# Compose runs inside the updater while controlling the host daemon. Its host
# project directory must be explicit or relative binds become /stack/* on host.
export HOST_STACK_DIR="$test_directory"
export COMPOSE_PROBE="${test_directory}/compose-probe.log"
# Invoked indirectly by compose_stack.
# shellcheck disable=SC2317,SC2329
docker() { printf 'env=%s args=%s\n' "$COMPOSE_ENV_FILE" "$*" >"$COMPOSE_PROBE"; }
compose_stack ps updater
assert grep -Fq -- "--project-directory $test_directory" "$COMPOSE_PROBE"
assert grep -Fq -- "env=$SNAPDOG_ENV_FILE" "$COMPOSE_PROBE"
unset -f docker

# Server releases deliberately own GitHub's Latest badge. Keep both the control
# API and the actual deployment path on the bounded single-release endpoint.
assert test "$RELEASE_API" = "https://api.github.com/repos/SnapDogRocks/snapdog/releases/latest"

# Docker materializes a missing bind source as a directory. Reject it during
# preflight, before a backup or deployment attempt.
# Variables expand in the inner shell.
# shellcheck disable=SC2016
if env COMPOSE_FILE="$test_directory" SNAPDOG_ENV_FILE="$SNAPDOG_ENV_FILE" \
    HOST_STACK_DIR="$test_directory" UPDATE_SH="$(dirname "${BASH_SOURCE[0]}")/update.sh" \
    bash -c 'source "$UPDATE_SH"; require_command() { :; }; validate_config' >/dev/null 2>&1; then
  printf 'FAILED: updater accepted a directory as its compose file\n' >&2
  exit 1
fi

assert version_is_newer v1.8.1 v1.8.2
assert version_is_newer 1.8.9 v1.9.0
assert version_is_newer v1.9.9 v2.0.0
if version_is_newer v1.8.2 v1.8.2; then exit 1; fi
if version_is_newer v2.0.0 v1.9.9; then exit 1; fi

# Invoked indirectly by latest_server_tag; ShellCheck versions use either code.
# shellcheck disable=SC2317,SC2329
github_curl() { printf '%s' '[{"tag_name":"client-v1.3.2","draft":false,"prerelease":false},{"tag_name":"v1.8.1","draft":false,"prerelease":false},{"tag_name":"v1.8.2","draft":false,"prerelease":false}]'; }
assert test "$(latest_server_tag)" = "v1.8.2"

# Once the control API has selected a release, deployment must use that exact
# validated target without depending on GitHub Releases a second time.
assert grep -Fq 'TARGET_VERSION: targetVersion' "$(dirname "${BASH_SOURCE[0]}")/control.mjs"

# shellcheck disable=SC2317,SC2329
github_curl() { printf '%s' '[{"tag_name":"client-v1.3.2","draft":false,"prerelease":false}]'; }
if latest_server_tag; then exit 1; fi

# Custom RELEASE_API endpoints returning one release remain supported.
# shellcheck disable=SC2317,SC2329
github_curl() { printf '%s' '{"tag_name":"v1.8.2","draft":false,"prerelease":false}'; }
assert test "$(latest_server_tag)" = "v1.8.2"

persist_server_image 'ghcr.io/snapdogrocks/snapdog:v1.8.2'
assert grep -Fx 'SNAPDOG_IMAGE=ghcr.io/snapdogrocks/snapdog:v1.8.2' "$SNAPDOG_ENV_FILE"
assert grep -Fx 'UPDATER_IMAGE=ghcr.io/snapdogrocks/snapdog-updater:v1.8.1' "$SNAPDOG_ENV_FILE"
assert grep -Fx 'SOME_SETTING=preserved' "$SNAPDOG_ENV_FILE"
assert grep -Fx 'SNAPDOG_IMAGE=ghcr.io/snapdogrocks/snapdog:v1.8.1' "$ENV_BACKUP_FILE"

printf 'updater shell tests passed\n'

# ── Updater self-update ───────────────────────────────────────────────
# The pin rewrite must be surgical: one key, others untouched.
assert persist_env_pin UPDATER_IMAGE 'ghcr.io/snapdogrocks/snapdog-updater:v1.8.2'
assert grep -Fqx 'UPDATER_IMAGE=ghcr.io/snapdogrocks/snapdog-updater:v1.8.2' "$SNAPDOG_ENV_FILE"
assert grep -Fqx 'SNAPDOG_IMAGE=ghcr.io/snapdogrocks/snapdog:v1.8.2' "$SNAPDOG_ENV_FILE"
assert grep -Fqx 'SOME_SETTING=preserved' "$SNAPDOG_ENV_FILE"

# Explicitly disabled policy: a server update must not swap the updater.
assert test "$AUTO_UPDATE_UPDATER" = "false"
assert schedule_updater_swap v1.9.0

# The enabled path needs its own process because the flag is readonly once sourced.
update_sh="$(dirname "${BASH_SOURCE[0]}")/update.sh"

swap_probe() {
  # `env` rather than a command prefix: the flag is readonly in this shell once
  # update.sh has been sourced, and bash rejects even a prefix assignment.
  # The single-quoted body is deliberate — those variables must expand in the
  # INNER shell, from the environment set here.
  # shellcheck disable=SC2016
  env AUTO_UPDATE_UPDATER=true UPDATER_VERSION="$1" UPDATE_ONCE=true \
    COMPOSE_FILE="$COMPOSE_FILE" SNAPDOG_ENV_FILE="$SNAPDOG_ENV_FILE" \
    HOST_STACK_DIR="$test_directory" \
    ENV_BACKUP_FILE="$ENV_BACKUP_FILE" SWAP_RESULT_FILE="${test_directory}/state/swap.json" \
    PROBE_LOG="$PROBE_LOG" UPDATE_SH="$update_sh" \
    bash -c '
    source "$UPDATE_SH"
    verify_image() { return 0; }
    updater_container_id() { printf "self-id"; }
    docker() {
      case "$1" in
        inspect) printf "ghcr.io/snapdogrocks/snapdog-updater:%s\n" "${UPDATER_VERSION#v}" ;;
        run) printf "%s\n" "$*" >>"$PROBE_LOG" ;;
      esac
    }
    schedule_updater_swap "$1" || true
  ' _ "$2"
}

export PROBE_LOG="${test_directory}/probe.log"
: >"$PROBE_LOG"
swap_result="${test_directory}/state/swap.json"
mkdir -p "$(dirname "$swap_result")"
printf '%s\n' '{"outcome":"rolled-back","detail":"stale","at":null}' >"$swap_result"

# Behind the release -> hand off to a detached helper for the right candidate.
swap_probe v1.8.1 v1.8.2
assert test ! -e "$swap_result"
assert grep -q -- '--swap-updater ghcr.io/snapdogrocks/snapdog-updater:v1.8.2' "$PROBE_LOG"
# Must inherit the mounts rather than pass container paths as host paths.
assert grep -q -- '--volumes-from self-id' "$PROBE_LOG"
# Must not reach the network while swapping.
assert grep -q -- '--network none' "$PROBE_LOG"
assert grep -q -- "--env HOST_STACK_DIR=$test_directory" "$PROBE_LOG"
assert grep -q -- "--env COMPOSE_ENV_FILE=$SNAPDOG_ENV_FILE" "$PROBE_LOG"

# Already current, and newer than the release: never launch a helper.
: >"$PROBE_LOG"
swap_probe v1.8.2 v1.8.2
swap_probe v1.9.9 v1.8.2
if [[ -s "$PROBE_LOG" ]]; then
  printf 'FAILED: helper launched despite an up-to-date updater\n' >&2
  exit 1
fi

# A helper launch failure is terminal and must replace any stale outcome with a
# result the new server can surface instead of leaving its overlay spinning.
# shellcheck disable=SC2016
env AUTO_UPDATE_UPDATER=true UPDATER_VERSION=v1.8.1 UPDATE_ONCE=true \
  COMPOSE_FILE="$COMPOSE_FILE" SNAPDOG_ENV_FILE="$SNAPDOG_ENV_FILE" \
  HOST_STACK_DIR="$test_directory" ENV_BACKUP_FILE="$ENV_BACKUP_FILE" \
  SWAP_RESULT_FILE="$swap_result" UPDATE_SH="$update_sh" \
  bash -c '
    source "$UPDATE_SH"
    verify_image() { return 0; }
    updater_container_id() { printf "self-id"; }
    docker() {
      [[ "$1" == "inspect" ]] && printf "ghcr.io/snapdogrocks/snapdog-updater:1.8.1\n"
      [[ "$1" != "run" ]]
    }
    if schedule_updater_swap v1.8.2; then exit 1; fi
  '
assert jq -e '.outcome == "failed" and (.detail | contains("detached updater swap helper"))' \
  "$swap_result" >/dev/null

printf 'updater self-update assertions passed\n'

# ── Phase journal ─────────────────────────────────────────────────────
# The server cannot narrate its own restart, so the journal is the only progress
# the UI can show. Pin its shape.
export PHASE_STARTED_AT="2026-08-13T00:00:00Z"
set_phase "deploying" "ghcr.io/snapdogrocks/snapdog:v1.8.2"
assert test -f "$PROGRESS_FILE"
assert jq -e '.phase == "deploying"' "$PROGRESS_FILE" >/dev/null
assert jq -e '.detail == "ghcr.io/snapdogrocks/snapdog:v1.8.2"' "$PROGRESS_FILE" >/dev/null
# startedAt must survive across phases so the UI can show elapsed time.
assert jq -e '.startedAt == "2026-08-13T00:00:00Z"' "$PROGRESS_FILE" >/dev/null
set_phase "done"
assert jq -e '.phase == "done" and .detail == null' "$PROGRESS_FILE" >/dev/null
assert jq -e '.startedAt == "2026-08-13T00:00:00Z"' "$PROGRESS_FILE" >/dev/null

# Failure metadata must retain the original failed step while rollback advances,
# so the UI can render both outcomes instead of collapsing everything to failed.
set_phase "verifying"
set_phase "backing-up"
set_phase "deploying"
set_phase "rolling-back" "rollback image"
assert jq -e '.phase == "rolling-back" and .failedPhase == "deploying" and .rollbackAttempted == true' "$PROGRESS_FILE" >/dev/null
set_phase "failed" "new release was rolled back"
assert jq -e '.phase == "failed" and .failedPhase == "deploying" and .rollbackAttempted == true' "$PROGRESS_FILE" >/dev/null

# A journal write must never abort an update, even if the path is unwritable.
# Subshell because PROGRESS_FILE is readonly once update.sh has been sourced.
# shellcheck disable=SC2016
if ! env PROGRESS_FILE=/proc/nonexistent/progress.json UPDATE_SH="$update_sh" \
     bash -c 'source "$UPDATE_SH"; set_phase "verifying"' >/dev/null 2>&1; then
  printf 'FAILED: set_phase must stay advisory when the journal is unwritable\n' >&2
  exit 1
fi

printf 'phase journal assertions passed\n'
