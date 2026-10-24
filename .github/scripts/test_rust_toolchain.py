"""rust-toolchain.toml is the only place that chooses the Rust version.

Workflows install exactly that toolchain, and the Rust container images (which
cannot read the file) name the same version and are pinned by digest. Dependabot
updates the file and the images in separate pull requests, so this keeps them
from drifting apart.
"""

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
IMAGE = re.compile(r"\brust:([0-9][0-9.]*)-[a-z]+(@sha256:[0-9a-f]{64})?")


def channel() -> str:
    text = (ROOT / "rust-toolchain.toml").read_text()
    match = re.search(r'^channel = "([^"]+)"$', text, re.MULTILINE)
    if match is None:
        raise AssertionError("rust-toolchain.toml has no channel")
    return match.group(1)


def files_with_images():
    yield from sorted((ROOT / ".github/workflows").glob("*.yml"))
    yield ROOT / "Dockerfile"
    yield ROOT / "Dockerfile.client"
    yield from sorted((ROOT / "devcontainer").glob("**/Dockerfile*"))


class RustToolchainTests(unittest.TestCase):
    def test_workflows_install_the_toolchain_file(self):
        for workflow in sorted((ROOT / ".github/workflows").glob("*.yml")):
            text = workflow.read_text()
            with self.subTest(workflow=workflow.name):
                self.assertNotIn("dtolnay/rust-toolchain", text)
                self.assertNotRegex(text, r"rustup (default|toolchain install \S)")
                self.assertNotRegex(text, r"cargo \+[0-9]")

    def test_rust_images_name_the_toolchain_version_and_a_digest(self):
        version = channel()
        seen = 0
        for path in files_with_images():
            for match in IMAGE.finditer(path.read_text()):
                seen += 1
                with self.subTest(file=str(path.relative_to(ROOT)), image=match.group(0)):
                    self.assertEqual(match.group(1), version)
                    self.assertIsNotNone(match.group(2), "image not pinned by digest")
        self.assertGreater(seen, 0, "no Rust image found; is the pattern stale?")


if __name__ == "__main__":
    unittest.main()
