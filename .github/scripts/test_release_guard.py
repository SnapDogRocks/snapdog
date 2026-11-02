import json
import tempfile
import unittest
from pathlib import Path

import release_guard

DEPRECATED = {"components": {"schemas": {"PlaylistInfo": {"properties": {
    "id": {"type": "integer", "deprecated": True}}}}}}
CLEAN = {"components": {"schemas": {"PlaylistInfo": {"properties": {
    "index": {"type": "integer"}}}}}}
# The version of the document, not of the product. Must never block anything.
DOCUMENT_VERSION = {"info": {"version": "1.0.0"}}


class ReleaseGuardTests(unittest.TestCase):
    def check(self, version, openapi, asyncapi=None, cargo="0.30.0", manifest="0.30.0"):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "docs/api").mkdir(parents=True)
            (root / "openapi.json").write_text(json.dumps(openapi))
            (root / "docs/api/asyncapi.json").write_text(json.dumps(asyncapi or CLEAN))
            (root / "Cargo.toml").write_text(f'[workspace.package]\nversion = "{cargo}"\n')
            (root / ".release-please-manifest.json").write_text(json.dumps({".": manifest}))
            argv = ["--root", str(root)] + (["--version", version] if version else [])
            return release_guard.main(argv)

    def test_a_release_candidate_with_a_deprecated_entry_is_refused(self):
        self.assertEqual(self.check("1.0.0-rc.1", DEPRECATED), 1)

    def test_every_one_x_version_is_refused(self):
        for version in ("1.0.0", "1.0.0-rc.1", "1.0.0-beta.2", "1.4.7", "2.0.0-rc.1"):
            self.assertEqual(self.check(version, DEPRECATED), 1, version)

    def test_a_deprecated_entry_in_the_asyncapi_document_refuses_too(self):
        self.assertEqual(self.check("1.0.0", CLEAN, asyncapi=DEPRECATED), 1)

    def test_zero_x_versions_are_allowed_with_deprecated_entries(self):
        for version in ("0.30.0", "0.99.9", "0.31.0-rc.1"):
            self.assertEqual(self.check(version, DEPRECATED), 0, version)

    def test_a_release_candidate_without_deprecated_entries_is_allowed(self):
        self.assertEqual(self.check("1.0.0-rc.1", CLEAN), 0)

    def test_the_version_of_the_document_is_not_the_product_version(self):
        self.assertEqual(self.check(None, DOCUMENT_VERSION), 0)
        self.assertEqual(self.check("0.30.0", DOCUMENT_VERSION), 0)
        # ... and a 1.x release is not blocked by it either.
        self.assertEqual(self.check("1.0.0", DOCUMENT_VERSION), 0)

    def test_the_product_version_is_the_larger_of_cargo_and_the_manifest(self):
        # The release pull request bumps both, a hand edit may bump one.
        self.assertEqual(self.check(None, DEPRECATED, cargo="0.30.0", manifest="1.0.0-rc.1"), 1)
        self.assertEqual(self.check(None, DEPRECATED, cargo="1.0.0", manifest="0.30.0"), 1)
        self.assertEqual(self.check(None, DEPRECATED, cargo="0.30.0", manifest="0.30.1"), 0)

    def test_what_cannot_be_read_never_passes(self):
        self.assertEqual(self.check("not-a-version", CLEAN), 2)
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(release_guard.main(["--root", directory, "--version", "1.0.0"]), 2)

    def test_the_repository_as_it_is_passes(self):
        self.assertEqual(release_guard.main([]), 0)


if __name__ == "__main__":
    unittest.main()
