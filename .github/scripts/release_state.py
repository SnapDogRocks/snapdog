#!/usr/bin/env python3
"""Read release state without using the published-only tag endpoint."""

import argparse
import json
import subprocess
import sys


def api(*arguments):
    return json.loads(subprocess.check_output(["gh", "api", *arguments], text=True))


def validate(release, tag, draft, release_id=None, empty=False):
    if not isinstance(release, dict):
        raise ValueError("Invalid release response")
    actual_id = release.get("id")
    # bool is an int subclass in Python, but cannot represent a release ID.
    if not isinstance(actual_id, int) or isinstance(actual_id, bool) or actual_id <= 0:
        raise ValueError("Invalid release ID")
    if release_id is not None and actual_id != release_id:
        raise ValueError("Release ID changed")
    if release.get("tag_name") != tag or release.get("draft") is not draft:
        raise ValueError("Unexpected release tag or draft state")
    assets = release.get("assets")
    if not isinstance(assets, list) or (len(assets) == 0) != empty:
        raise ValueError("Expected an empty draft" if empty else "Expected release assets")
    if not isinstance(release.get("prerelease"), bool):
        raise ValueError("Invalid prerelease state")
    return release


def resolve(repository, tag):
    pages = api("--paginate", "--slurp", f"repos/{repository}/releases?per_page=100")
    if not isinstance(pages, list) or not all(isinstance(page, list) for page in pages):
        raise ValueError("Invalid paginated release list")
    releases = [release for page in pages for release in page]
    if not all(isinstance(release, dict) for release in releases):
        raise ValueError("Invalid release list entry")
    matches = [release for release in releases if release.get("tag_name") == tag]
    # Count every matching release, including published ones. Never select the
    # first draft and silently ignore an ambiguous/published match.
    if len(matches) != 1:
        raise ValueError(f"Expected exactly one release for {tag}, found {len(matches)}")
    return validate(matches[0], tag, draft=True, empty=True)


def verify(repository, tag, release_id, draft):
    release = api(f"repos/{repository}/releases/{release_id}")
    return validate(release, tag, draft=draft, release_id=release_id)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("resolve", "verify"):
        sub = commands.add_parser(command)
        sub.add_argument("repository")
        sub.add_argument("tag")
        if command == "verify":
            sub.add_argument("release_id", type=int)
            sub.add_argument("state", choices=("draft", "published"))
    args = parser.parse_args()
    try:
        if args.command == "resolve":
            release = resolve(args.repository, args.tag)
        else:
            release = verify(args.repository, args.tag, args.release_id, args.state == "draft")
        print(json.dumps(release))
    except (ValueError, subprocess.CalledProcessError) as error:
        print(f"Release state check failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
