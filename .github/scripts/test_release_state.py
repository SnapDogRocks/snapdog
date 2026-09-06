import copy
import json
import subprocess
import unittest
from unittest.mock import patch

import release_state


REPOSITORY = "SnapDogRocks/snapdog"
TAG = "v0.27.2"
DRAFT = {"id": 383526531, "tag_name": TAG, "draft": True, "prerelease": False, "assets": []}


class ReleaseStateTests(unittest.TestCase):
    def setUp(self):
        self.release = copy.deepcopy(DRAFT)

    def resolve(self, pages):
        with patch.object(release_state.subprocess, "check_output", return_value=json.dumps(pages)) as request:
            result = release_state.resolve(REPOSITORY, TAG)
        request.assert_called_once_with(
            ["gh", "api", "--paginate", "--slurp", f"repos/{REPOSITORY}/releases?per_page=100"],
            text=True,
        )
        return result

    def test_finds_empty_draft_on_later_page(self):
        older = dict(self.release, id=1, tag_name="v0.27.1", draft=False)
        self.assertEqual(self.resolve([[older], [], [self.release]]), self.release)

    def test_missing_release(self):
        with self.assertRaisesRegex(ValueError, "exactly one"):
            self.resolve([[]])

    def test_duplicate_drafts(self):
        with self.assertRaisesRegex(ValueError, "exactly one"):
            self.resolve([[self.release], [dict(self.release, id=2)]])

    def test_draft_and_published_duplicate(self):
        with self.assertRaisesRegex(ValueError, "exactly one"):
            self.resolve([[self.release, dict(self.release, id=2, draft=False)]])

    def test_published_release(self):
        with self.assertRaisesRegex(ValueError, "draft state"):
            self.resolve([[dict(self.release, draft=False)]])

    def test_existing_assets_reject_retry(self):
        with self.assertRaisesRegex(ValueError, "empty draft"):
            self.resolve([[dict(self.release, assets=[{"id": 1}])]])

    def test_malformed_pagination(self):
        for response in ({}, [None], [[None]]):
            with self.subTest(response=response), self.assertRaises(ValueError):
                self.resolve(response)

    def test_invalid_or_missing_fields(self):
        for key, values in {
            "id": [None, 0, -1, True, "123", 1.5],
            "draft": [None, "true", 1],
            "assets": [None, {}],
            "prerelease": [None, "false", 0],
        }.items():
            for value in values:
                with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                    self.resolve([[dict(self.release, **{key: value})]])
            release = dict(self.release)
            del release[key]
            with self.subTest(missing=key), self.assertRaises(ValueError):
                self.resolve([[release]])

    def test_prerelease_allowed(self):
        self.assertTrue(self.resolve([[dict(self.release, prerelease=True)]])["prerelease"])

    def test_api_failure_propagates(self):
        with patch.object(release_state.subprocess, "check_output", side_effect=subprocess.CalledProcessError(1, "gh")):
            with self.assertRaises(subprocess.CalledProcessError):
                release_state.resolve(REPOSITORY, TAG)

    def test_invalid_json_rejected(self):
        with patch.object(release_state.subprocess, "check_output", return_value="not json"):
            with self.assertRaises(ValueError):
                release_state.resolve(REPOSITORY, TAG)

    def verify(self, release, draft):
        with patch.object(release_state.subprocess, "check_output", return_value=json.dumps(release)) as request:
            result = release_state.verify(REPOSITORY, TAG, DRAFT["id"], draft)
        request.assert_called_once_with(
            ["gh", "api", f"repos/{REPOSITORY}/releases/{DRAFT['id']}"], text=True
        )
        return result

    def test_verify_by_id_before_and_after_publish(self):
        for draft in (True, False):
            release = dict(self.release, draft=draft, assets=[{"id": 1}])
            self.assertEqual(self.verify(release, draft), release)

    def test_verify_rejects_replacement_release(self):
        with self.assertRaisesRegex(ValueError, "ID changed"):
            self.verify(dict(self.release, id=2, assets=[{"id": 1}]), True)

    def test_verify_rejects_wrong_tag(self):
        with self.assertRaisesRegex(ValueError, "tag"):
            self.verify(dict(self.release, tag_name="v0.27.3", assets=[{"id": 1}]), True)

    def test_verify_rejects_wrong_publication_state(self):
        for draft in (True, False):
            with self.subTest(draft=draft), self.assertRaisesRegex(ValueError, "draft state"):
                self.verify(dict(self.release, draft=not draft, assets=[{"id": 1}]), draft)

    def test_verify_rejects_empty_assets(self):
        with self.assertRaisesRegex(ValueError, "Expected release assets"):
            self.verify(self.release, True)

    def test_verify_api_failure_propagates(self):
        with patch.object(release_state.subprocess, "check_output", side_effect=subprocess.CalledProcessError(1, "gh")):
            with self.assertRaises(subprocess.CalledProcessError):
                release_state.verify(REPOSITORY, TAG, DRAFT["id"], True)


if __name__ == "__main__":
    unittest.main()
