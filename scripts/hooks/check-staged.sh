#!/bin/sh
# Fast pre-commit checks that do not have to compile.
# Anything taking longer than a second or two belongs in pre-push.
set -eu

max_bytes=${MAX_STAGED_BYTES:-5242880}   # 5 MiB
status=0

fail() { printf '\033[31m%s\033[0m %s\n' 'Commit rejected:' "$1" >&2; status=1; }

# --- Direct commit to the default branch ----------------------------------
# symbolic-ref rather than rev-parse: it also works on a branch that is still
# unborn, that is in a repository without a first commit. rev-parse only
# returns "HEAD" there and the guard would not bite.
branch=$(git symbolic-ref --quiet --short HEAD 2>/dev/null || echo '')
default=$(git symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null |
    sed 's#^origin/##')
default=${default:-main}
if [ "$branch" = "$default" ] && [ "${ALLOW_COMMIT_ON_DEFAULT:-}" != '1' ]; then
    fail "Direct commit to '$default'. Create a branch, or set ALLOW_COMMIT_ON_DEFAULT=1."
fi

# --- Large files ----------------------------------------------------------
# Catches accidentally added archives, images, DMGs and key files before they
# stand in the history irrevocably.
# `read -d` is a bashism and fails under dash; hence line by line.
# core.quotePath=false is mandatory: otherwise git returns non-ASCII paths
# quoted and escaped ("gr\303\266e.bin"), `[ -f ]` does not find the file
# and the guard lets it through silently. Verified in practice.
git -c core.quotePath=false diff --cached --name-only --diff-filter=AM | while IFS= read -r file; do
    [ -f "$file" ] || continue
    size=$(wc -c < "$file" | tr -d ' ')
    if [ "$size" -gt "$max_bytes" ]; then
        printf '\033[31mCommit rejected:\033[0m %s is %s bytes (limit %s).\n' \
            "$file" "$size" "$max_bytes" >&2
        printf '  Does the file belong in the repository? If not, extend .gitignore.\n' >&2
        printf '  Deliberate exception: raise MAX_STAGED_BYTES.\n' >&2
        exit 1
    fi
done || status=1

# --- Directories that never belong in a commit ----------------------------
if git -c core.quotePath=false diff --cached --name-only |
    grep -Eq '(^|/)(node_modules|target|dist|build|\.next|coverage)/'; then
    fail "A build or dependency directory is staged. Check .gitignore."
fi

exit "$status"
