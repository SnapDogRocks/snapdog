"""Tests for check-contribution-license.py (the file name has hyphens, so it is loaded by path)."""
import importlib.util
import pathlib
import unittest

_SPEC = importlib.util.spec_from_file_location(
    "check_contribution_license", pathlib.Path(__file__).with_name("check-contribution-license.py")
)
mod = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(mod)

BOX = "- [x] I have read CONTRIBUTION-LICENSE.md (CONTRIBUTION-LICENSE v1) and license this contribution."
OWNERS = {"metaneutrons"}


def pr(login="alice", body=BOX, kind="User"):
    return {"user": {"login": login, "type": kind}, "body": body}


def commit(login="alice", email="alice@example.org", message="fix: x", sha="abcdef1234"):
    return {
        "sha": sha,
        "author": {"login": login},
        "commit": {"message": message, "author": {"email": email}},
    }


class DeclarationTests(unittest.TestCase):
    def test_ticked_box_passes(self):
        self.assertEqual(mod.check(pr(), [commit()], OWNERS), [])

    def test_missing_box_fails(self):
        self.assertTrue(mod.check(pr(body="No declaration."), [commit()], OWNERS))

    def test_unticked_box_fails(self):
        body = BOX.replace("[x]", "[ ]")
        self.assertTrue(mod.check(pr(body=body), [commit()], OWNERS))

    def test_empty_body_fails(self):
        self.assertTrue(mod.check(pr(body=None), [commit()], OWNERS))

    def test_box_without_marker_fails(self):
        self.assertTrue(mod.check(pr(body="- [x] I agree"), [commit()], OWNERS))

    def test_bot_author_passes_without_box(self):
        self.assertEqual(mod.check(pr("dependabot[bot]", body="", kind="Bot"), [commit("dependabot[bot]")], OWNERS), [])

    def test_maintainer_passes_without_box(self):
        self.assertEqual(mod.check(pr("metaneutrons", body=""), [commit("metaneutrons")], OWNERS), [])

    def test_commit_by_another_person_needs_sign_off(self):
        other = commit("bob", "bob@example.org")
        self.assertTrue(mod.check(pr(), [commit(), other], OWNERS))

    def test_commit_by_another_person_with_sign_off_passes(self):
        other = commit("bob", "bob@example.org", "fix: y\n\nSigned-off-by: Bob <bob@example.org>")
        self.assertEqual(mod.check(pr(), [commit(), other], OWNERS), [])

    def test_sign_off_with_other_email_does_not_cover_the_commit(self):
        other = commit("bob", "bob@example.org", "fix: y\n\nSigned-off-by: Bob <other@example.org>")
        self.assertTrue(mod.check(pr(), [other], OWNERS))

    def test_co_author_needs_sign_off(self):
        msg = "fix: z\n\nCo-authored-by: Carol <carol@example.org>"
        self.assertTrue(mod.check(pr(), [commit(message=msg)], OWNERS))

    def test_co_author_with_sign_off_passes(self):
        msg = "fix: z\n\nCo-authored-by: Carol <carol@example.org>\nSigned-off-by: Carol <carol@example.org>"
        self.assertEqual(mod.check(pr(), [commit(message=msg)], OWNERS), [])

    def test_bot_co_author_passes(self):
        msg = "fix: z\n\nCo-authored-by: dependabot[bot] <bot@example.org>"
        self.assertEqual(mod.check(pr(), [commit(message=msg)], OWNERS), [])

    def test_commit_by_maintainer_in_foreign_pr_passes(self):
        owner = commit("metaneutrons", "owner@example.org")
        self.assertEqual(mod.check(pr(), [commit(), owner], OWNERS), [])


if __name__ == "__main__":
    unittest.main()
