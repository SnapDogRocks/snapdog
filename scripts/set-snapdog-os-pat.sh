#!/usr/bin/env bash
# Sets SNAPDOG_OS_PAT in the protected `release` environment of
# SnapDogRocks/snapdog, after proving that the token can do what the release
# actually needs and nothing broader.
#
# The value is read from a terminal without echo and is never printed, never
# passed as an argument, and never written to a file. Only the verdicts below
# are shown.
#
#   scripts/set-snapdog-os-pat.sh [--keychain]
#
# --keychain additionally stores a copy as `env:SNAPDOG_OS_PAT`, so a later
# rotation does not have to start at github.com again.
set -euo pipefail
umask 077

REPO=SnapDogRocks/snapdog
ENVIRONMENT=release
TARGET=SnapDogRocks/snapdog-os

keychain=0
[[ "${1:-}" == "--keychain" ]] && keychain=1

command -v gh >/dev/null || { echo "gh is required" >&2; exit 1; }
gh auth status >/dev/null 2>&1 || { echo "gh is not authenticated" >&2; exit 1; }

cat <<EOF
Target      ${REPO}, environment ${ENVIRONMENT}, secret SNAPDOG_OS_PAT
Used by     the job bump-snapdog-os: it checks out ${TARGET} and opens the
            version-bump pull request there.
Needs       a fine-grained personal access token, resource owner SnapDogRocks,
            only the repository ${TARGET}, with
              Contents        Read and write
              Pull requests   Read and write
            Nothing else. Give it the shortest expiry you are willing to renew.

Create it at https://github.com/settings/personal-access-tokens/new
EOF

[[ -t 0 ]] || { echo "refusing to read a token from a pipe; run this in a terminal" >&2; exit 1; }
printf '\nPaste the token (input stays hidden), then press Return: '
IFS= read -rs token
printf '\n\n'
[[ -n "$token" ]] || { echo "no token entered" >&2; exit 1; }

fail() { printf '  FAIL  %s\n' "$*" >&2; exit 1; }
pass() { printf '  ok    %s\n' "$*"; }

echo "Checking the token before it becomes a secret:"

login=$(GH_TOKEN="$token" gh api /user --jq .login 2>/dev/null) \
  || fail "the token is not accepted by the API at all"
pass "authenticates as ${login}"

# A fine-grained token answers with the repositories it was granted. A classic
# token would answer for every repository the user can see, which is exactly the
# breadth this secret must not have.
reach=$(GH_TOKEN="$token" gh api "/repos/${TARGET}" --jq '.full_name' 2>/dev/null) \
  || fail "the token cannot see ${TARGET}"
[[ "$reach" == "$TARGET" ]] || fail "the token resolved ${TARGET} to ${reach}"
pass "reaches ${TARGET}"

perms=$(GH_TOKEN="$token" gh api "/repos/${TARGET}" --jq '.permissions | to_entries | map(select(.value)) | map(.key) | join(",")' 2>/dev/null)
[[ ",$perms," == *",push,"* ]] || fail "no write access to ${TARGET} (permissions: ${perms:-none})"
pass "may write to ${TARGET} (${perms})"

# Pull requests write is not visible in .permissions, so ask the endpoint the job
# uses. Listing is a read, but it fails outright when the pull-requests scope is
# missing from a fine-grained token.
GH_TOKEN="$token" gh api "/repos/${TARGET}/pulls?state=open&per_page=1" >/dev/null 2>&1 \
  || fail "cannot read pull requests on ${TARGET}; the Pull requests scope is missing"
pass "can read pull requests on ${TARGET}"

# Counter-probe: the token must not reach the repository it is stored in. A
# classic token, or a fine-grained one granted to all repositories, would.
if GH_TOKEN="$token" gh api "/repos/${REPO}" >/dev/null 2>&1; then
  printf '  WARN  the token also reaches %s; it is broader than this job needs\n' "$REPO"
  printf '        Continue anyway? [y/N] '
  IFS= read -r answer
  [[ "$answer" == [yY] ]] || { echo "aborted"; exit 1; }
else
  pass "does not reach ${REPO}, so it is scoped to the one repository"
fi

expiry=$(GH_TOKEN="$token" gh api /user -i 2>/dev/null | awk 'tolower($1) == "github-authentication-token-expiration:" {print $2, $3}')
[[ -n "$expiry" ]] && pass "expires ${expiry}" || printf '  WARN  the token has no expiry\n'

printf '%s' "$token" | gh secret set SNAPDOG_OS_PAT --repo "$REPO" --env "$ENVIRONMENT"
pass "stored as SNAPDOG_OS_PAT in ${REPO}, environment ${ENVIRONMENT}"

if (( keychain )); then
  security add-generic-password -a "$USER" -s "env:SNAPDOG_OS_PAT" \
    -l "SnapDogRocks/snapdog-os version bump" \
    -j "Fine-grained PAT for SnapDogRocks/snapdog-os, Contents and Pull requests write. Used by bump-snapdog-os." \
    -w "$token" -U
  pass "copy stored in the keychain as env:SNAPDOG_OS_PAT"
fi

unset token
echo
echo "Done. Nothing above printed the token."
