from __future__ import annotations

import os
import unittest
from unittest.mock import patch

from moli_cdp_smoke.runner import (
    ALL_GROUPS,
    DEFAULT_GROUP_NAMES,
    DEFAULT_GROUPS,
    GROUPS_BY_NAME,
    MANAGED_EXTERNAL_GROUPS,
    OPTIONAL_EXTERNAL_GROUPS,
    group_listing,
    resolve_group_selection,
)


class GroupSelectionTests(unittest.TestCase):
    def test_default_runs_every_repository_managed_group(self) -> None:
        with patch.dict(os.environ, {"MOLI_SMOKE_GROUPS": ""}):
            selection = resolve_group_selection()

        self.assertEqual(
            tuple(group.name for group in selection.groups),
            DEFAULT_GROUP_NAMES,
        )
        self.assertEqual(DEFAULT_GROUP_NAMES, tuple(group.name for group in DEFAULT_GROUPS))
        self.assertIn("debugger-breakpoints", DEFAULT_GROUP_NAMES)
        self.assertIn("runtime-exception", DEFAULT_GROUP_NAMES)
        self.assertIn("file-chooser", DEFAULT_GROUP_NAMES)
        self.assertIn("inspector-routing", DEFAULT_GROUP_NAMES)
        self.assertIn("navigation-outcomes", DEFAULT_GROUP_NAMES)
        self.assertIn("media-error", DEFAULT_GROUP_NAMES)
        self.assertIn("multi-page", DEFAULT_GROUP_NAMES)
        self.assertIn("puppeteer", DEFAULT_GROUP_NAMES)
        self.assertEqual(
            tuple(group.name for group in MANAGED_EXTERNAL_GROUPS),
            ("puppeteer",),
        )

    def test_only_external_environment_groups_remain_opt_in(self) -> None:
        optional_names = tuple(group.name for group in OPTIONAL_EXTERNAL_GROUPS)

        self.assertEqual(
            optional_names,
            ("chrome-remote-interface", "cdp-use", "stagehand", "agent-browser"),
        )
        self.assertTrue(set(optional_names).isdisjoint(DEFAULT_GROUP_NAMES))

    def test_registry_and_listing_mark_exact_default_set(self) -> None:
        self.assertEqual(len(ALL_GROUPS), len(GROUPS_BY_NAME))
        listed_defaults = tuple(
            group["name"] for group in group_listing() if group["default"]
        )

        self.assertEqual(listed_defaults, DEFAULT_GROUP_NAMES)

    def test_explicit_cli_and_environment_selection_stay_focused(self) -> None:
        explicit = resolve_group_selection(["protocol,puppeteer", "protocol"])
        self.assertEqual(
            tuple(group.name for group in explicit.groups),
            ("protocol", "puppeteer"),
        )

        with patch.dict(os.environ, {"MOLI_SMOKE_GROUPS": "action-window,core"}):
            from_environment = resolve_group_selection()
        self.assertEqual(
            tuple(group.name for group in from_environment.groups),
            ("action-window", "core"),
        )


if __name__ == "__main__":
    unittest.main()
