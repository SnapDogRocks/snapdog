#!/usr/bin/env python3
"""Fail if the macOS app's String Catalog isn't fully translated into the required locales.

Runs in CI (and can be used as a pre-commit/pre-push hook) so the SnapDog Server app's
localization never drifts out of completeness. A string passes when, for every required
locale, it has a `stringUnit` in state "translated" — or when it is explicitly marked
`shouldTranslate: false` (source-only by design).

Note: this checks the *catalog*. New UI strings enter the catalog when Xcode extracts them
on the next build/edit (added as "new"/untranslated), at which point this check flags them.
"""
import json
import sys
from pathlib import Path

CATALOG = Path(__file__).parent / "SnapDogServer" / "SnapDog Server" / "Localizable.xcstrings"
REQUIRED_LOCALES = ["de"]


def main() -> int:
    if not CATALOG.exists():
        print(f"::error::String catalog not found: {CATALOG}")
        return 1

    data = json.loads(CATALOG.read_text(encoding="utf-8"))
    strings = data.get("strings", {})
    missing: dict[str, list[str]] = {loc: [] for loc in REQUIRED_LOCALES}

    for key, entry in strings.items():
        if entry.get("shouldTranslate") is False:
            continue  # intentionally source-only
        for loc in REQUIRED_LOCALES:
            unit = entry.get("localizations", {}).get(loc, {}).get("stringUnit", {})
            if unit.get("state") != "translated" or not unit.get("value"):
                missing[loc].append(key)

    failed = False
    for loc, keys in missing.items():
        if keys:
            failed = True
            print(f"::error::{len(keys)} string(s) not translated to '{loc}' in {CATALOG.name}:")
            for k in sorted(keys):
                print(f"  - {k!r}")

    if failed:
        print("\nTranslate them in Xcode (or the .xcstrings) and re-run.")
        return 1

    print(f"i18n OK: all {len(strings)} strings translated to {', '.join(REQUIRED_LOCALES)}.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
