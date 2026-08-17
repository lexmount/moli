from __future__ import annotations

import json
import unittest
import urllib.request

from moli_cdp_smoke.fixture import FixtureServer
from moli_cdp_smoke.runner import DEFAULT_GROUP_NAMES, GROUPS_BY_NAME


class ActionWindowGroupTests(unittest.TestCase):
    def test_group_is_a_default_raw_contract(self) -> None:
        group = GROUPS_BY_NAME["action-window"]

        self.assertEqual(group.phase, "raw")
        self.assertTrue(group.default)
        self.assertIn(group.name, DEFAULT_GROUP_NAMES)


class ActionWindowFixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.fixture = FixtureServer()
        cls.fixture.start()

    @classmethod
    def tearDownClass(cls) -> None:
        cls.fixture.stop()

    def read(self, path: str) -> tuple[str, str]:
        with urllib.request.urlopen(f"{self.fixture.url}{path}", timeout=2) as response:
            return response.headers.get_content_type(), response.read().decode("utf-8")

    def read_json(self, path: str) -> dict[str, object]:
        content_type, body = self.read(path)
        self.assertEqual(content_type, "application/json")
        value = json.loads(body)
        self.assertIsInstance(value, dict)
        return value

    def test_witness_counter_resets_and_counts_entries(self) -> None:
        self.assertEqual(self.read_json("/action-window-witness/reset"), {"count": 0})
        self.assertEqual(self.read_json("/action-window-witness/status"), {"count": 0})
        self.assertEqual(self.read_json("/action-window-witness/entered"), {"count": 1})
        self.assertEqual(self.read_json("/action-window-witness/status"), {"count": 1})

    def test_action_window_pages_expose_each_contract_fixture(self) -> None:
        expected_markers = {
            "/action-window-deadline": "__actionWindowObserver",
            "/action-window-capture": "__actionWindowCaptureDeltas",
            "/action-window-replacement": "__actionWindowReplacementDeltas",
        }

        for path, marker in expected_markers.items():
            with self.subTest(path=path):
                content_type, body = self.read(path)
                self.assertEqual(content_type, "text/html")
                self.assertIn(marker, body)


if __name__ == "__main__":
    unittest.main()
