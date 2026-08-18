from __future__ import annotations

import os
import unittest
from unittest.mock import patch

from moli_cdp_smoke.runner import resolve_group_selection


class SmokeSelectionTests(unittest.TestCase):
    def test_inspector_routing_is_in_default_selection(self) -> None:
        with patch.dict(os.environ, {"MOLI_SMOKE_GROUPS": ""}):
            selection = resolve_group_selection()

        self.assertIn("inspector-routing", [group.name for group in selection.groups])


if __name__ == "__main__":
    unittest.main()
