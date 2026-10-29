from __future__ import annotations

import os
import unittest
from unittest.mock import patch

from moli_webdriver_smoke.runner import (
    ALL_GROUPS,
    CHROMEDRIVER_ORACLE_GROUPS,
    DEFAULT_GROUP_NAMES,
    DEFAULT_GROUPS,
    GROUPS_BY_NAME,
    MOLI_GROUPS,
    group_listing,
    resolve_group_selection,
)


class GroupSelectionTests(unittest.TestCase):
    def test_default_runs_every_moli_group(self) -> None:
        with patch.dict(os.environ, {"MOLI_WEBDRIVER_SMOKE_GROUPS": ""}):
            selection = resolve_group_selection()

        self.assertEqual(
            tuple(group.name for group in selection),
            DEFAULT_GROUP_NAMES,
        )
        self.assertEqual(DEFAULT_GROUPS, MOLI_GROUPS)
        self.assertIn("classic", DEFAULT_GROUP_NAMES)
        self.assertIn("bidi", DEFAULT_GROUP_NAMES)
        self.assertIn("bidi-storage", DEFAULT_GROUP_NAMES)
        self.assertIn("selenium", DEFAULT_GROUP_NAMES)
        self.assertIn("script-interrupt", DEFAULT_GROUP_NAMES)

    def test_only_chromedriver_oracle_remains_opt_in(self) -> None:
        oracle_names = tuple(group.name for group in CHROMEDRIVER_ORACLE_GROUPS)

        self.assertEqual(oracle_names, ("script-timeout-chromium",))
        self.assertTrue(set(oracle_names).isdisjoint(DEFAULT_GROUP_NAMES))

    def test_registry_and_listing_mark_exact_default_set(self) -> None:
        self.assertEqual(len(ALL_GROUPS), len(GROUPS_BY_NAME))
        listed_defaults = tuple(
            group["name"] for group in group_listing() if group["default"]
        )

        self.assertEqual(listed_defaults, DEFAULT_GROUP_NAMES)

    def test_explicit_cli_and_environment_selection_stay_focused(self) -> None:
        explicit = resolve_group_selection(["bidi,selenium", "bidi"])
        self.assertEqual(
            tuple(group.name for group in explicit),
            ("bidi", "selenium"),
        )

        with patch.dict(
            os.environ,
            {"MOLI_WEBDRIVER_SMOKE_GROUPS": "classic,script-interrupt"},
        ):
            from_environment = resolve_group_selection()
        self.assertEqual(
            tuple(group.name for group in from_environment),
            ("classic", "script-interrupt"),
        )


if __name__ == "__main__":
    unittest.main()
