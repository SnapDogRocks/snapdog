#!/usr/bin/env python3
"""Refuse a 1.x release while the API description still has a deprecated entry.

MED-0011 section 2.8: `id` and `ZoneChanged.playlist` are deprecated and removed with
SnapDog 1.0, which breaks `/api/v1` for clients that still read them. No version of 1.0.0
or above may exist while any entry of `openapi.json` or `docs/api/asyncapi.json` is marked
`deprecated`. That includes `1.0.0-rc.N` and every other 1.x pre-release, because a
pre-release is a version a client can install.

The product version is the larger of the workspace version in `Cargo.toml` and the entry
`.` of `.release-please-manifest.json`. `info.version` of the OpenAPI document is the
version of the document and is never read.

Exit status: 0 allowed, 1 refused, 2 the check could not run (an unreadable file or a
version it cannot parse never counts as a pass).
"""

import argparse
import json
import re
import sys
import tomllib
from pathlib import Path

SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$")
SPECS = ("openapi.json", "docs/api/asyncapi.json")


def parse(version):
    match = SEMVER.match(version)
    if not match:
        raise ValueError(f"not a SemVer version: {version!r}")
    return tuple(int(part) for part in match.groups())


def product_version(root):
    cargo = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    manifest = json.loads((root / ".release-please-manifest.json").read_text())["."]
    return max((cargo, manifest), key=parse)


def deprecated_entries(node, path="$"):
    """JSON paths of every object that has `"deprecated": true`."""
    found = []
    if isinstance(node, dict):
        if node.get("deprecated") is True:
            found.append(path)
        for key, value in node.items():
            found.extend(deprecated_entries(value, f"{path}.{key}"))
    elif isinstance(node, list):
        for index, value in enumerate(node):
            found.extend(deprecated_entries(value, f"{path}[{index}]"))
    return found


def violations(version, documents):
    """Entries that block `version`. `documents` maps a name to its parsed JSON."""
    if parse(version)[0] < 1:
        return []
    return [
        f"{name}: {entry}"
        for name, document in documents.items()
        for entry in deprecated_entries(document)
    ]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--version", help="version to check (default: the product version)")
    arguments = parser.parse_args(argv)
    try:
        version = arguments.version or product_version(arguments.root)
        parse(version)
        documents = {
            name: json.loads((arguments.root / name).read_text()) for name in SPECS
        }
        blocking = violations(version, documents)
    except (OSError, ValueError, KeyError) as error:
        print(f"release guard could not run: {error}", file=sys.stderr)
        return 2
    if blocking:
        print(
            f"version {version} is refused: the API description still has deprecated "
            "entries. Remove them (MED-0011, WP-5b) before a 1.x release.",
            file=sys.stderr,
        )
        for entry in blocking:
            print(f"  {entry}", file=sys.stderr)
        return 1
    print(f"release guard: version {version} allowed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
