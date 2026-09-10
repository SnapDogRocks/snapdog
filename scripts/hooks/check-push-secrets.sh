#!/bin/sh
# gitleaks over the commits actually being pushed only.
# A full history scan on every push becomes unusably slow as the repository
# grows and teaches people to reach for --no-verify.
set -eu

command -v gitleaks >/dev/null 2>&1 || {
    printf '\033[31mPush rejected:\033[0m gitleaks is missing (brew install gitleaks).\n' >&2
    exit 1
}

# Determine the range, from precise to coarse.
range=''
if upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null); then
    range="${upstream}..HEAD"
elif head_ref=$(git symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null); then
    range="${head_ref}..HEAD"
fi

if [ -z "$range" ]; then
    # No remote reference, for example on the first push of a new repository.
    printf 'gitleaks: no remote reference, full scan.\n' >&2
    exec gitleaks git --redact --verbose .
fi

# An empty range means there is nothing new to check.
if [ -z "$(git rev-list --max-count=1 "$range" 2>/dev/null || true)" ]; then
    printf 'gitleaks: no new commits in %s.\n' "$range" >&2
    exit 0
fi

printf 'gitleaks: checking %s\n' "$range" >&2
exec gitleaks git --redact --verbose --log-opts="$range" .
