#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Point the floating tags (latest, MAJOR.MINOR, MAJOR) of REPO at the manifest that the
# version tag TAG already resolves to. Used by release.yml after a release is promoted.
# imagetools creates tags against the existing manifest list; it does not rebuild and does
# not re-push layers, so every floating tag resolves to the exact bytes already signed and
# attested under the version tag.
set -euo pipefail
: "${REPO:?REPO must name the image repository}"
: "${TAG:?TAG must name the version tag}"

version="${TAG#v}"
# The callers are already gated on a non-prerelease tag; this is the belt-and-braces check,
# because a floating tag pointing at a prerelease is exactly the state to prevent.
case "$version" in
  *-*) echo "::error::IMG003 refusing to move floating tags for prerelease ${TAG}"; exit 1 ;;
esac
major="${version%%.*}"
rest="${version#*.}"
minor="${major}.${rest%%.*}"

digest=$(docker buildx imagetools inspect "${REPO}:${TAG}" \
  --format '{{json .Manifest.Digest}}' | tr -d '"')
test -n "$digest" || { echo "::error::IMG001 cannot resolve ${REPO}:${TAG}"; exit 1; }

docker buildx imagetools create \
  --tag "${REPO}:latest" \
  --tag "${REPO}:${minor}" \
  --tag "${REPO}:${major}" \
  "${REPO}@${digest}"

for floating in latest "$minor" "$major"; do
  actual=$(docker buildx imagetools inspect "${REPO}:${floating}" \
    --format '{{json .Manifest.Digest}}' | tr -d '"')
  if [[ "$actual" != "$digest" ]]; then
    echo "::error::IMG002 ${REPO}:${floating} is ${actual}, expected ${digest}"
    exit 1
  fi
  echo "${REPO}:${floating} -> ${digest}"
done
