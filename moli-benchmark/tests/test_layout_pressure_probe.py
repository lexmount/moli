from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


def load_probe():
    path = (
        Path(__file__).resolve().parents[1]
        / "scripts/probe-layout-working-set-pressure.py"
    )
    spec = importlib.util.spec_from_file_location("layout_pressure_probe", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PressureEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.probe = load_probe()

    def complete_journal(self):
        rows = [{"event": "start", "data": {"schema": 2, "pages": 3, "rounds": 2}}]
        for round_index in range(2):
            for offset in range(3):
                rows.append(
                    {
                        "event": "grant",
                        "data": {
                            "round": round_index,
                            "page": (round_index + offset) % 3,
                        },
                    }
                )
                if offset:
                    rows.append({"event": "handoff_begin", "data": {}})
            rows.append(
                {
                    "event": "round_complete",
                    "data": {"round": round_index, "grants": [round_index + 1] * 3},
                }
            )
        rows.extend(
            {"event": "page_closed", "data": {"index": index}} for index in range(3)
        )
        rows.append({"event": "result", "data": {"ok": True}})
        return rows

    def test_complete_finite_service_is_accepted(self):
        self.assertEqual(
            self.probe.validate_scenario(self.complete_journal(), 3, 2), []
        )

    def test_success_marker_does_not_hide_a_missing_round_or_close(self):
        rows = self.complete_journal()
        rows = [
            row
            for row in rows
            if not (row["event"] == "round_complete" and row["data"]["round"] == 1)
        ]
        rows = [
            row
            for row in rows
            if not (row["event"] == "page_closed" and row["data"]["index"] == 2)
        ]
        errors = self.probe.validate_scenario(rows, 3, 2)
        self.assertTrue(any("rounds" in error for error in errors))
        self.assertTrue(any("close" in error for error in errors))

    def test_aggregate_totals_do_not_hide_one_page_losing_its_turn(self):
        rows = self.complete_journal()
        grant = next(
            row for row in rows if row["event"] == "grant" and row["data"]["page"] == 2
        )
        grant["data"]["page"] = 1
        self.assertTrue(
            any(
                "grant sequence" in error
                for error in self.probe.validate_scenario(rows, 3, 2)
            )
        )

    def test_original_two_page_evidence_remains_supported(self):
        rows = [
            {"event": "start", "data": {"schema": 1, "pages": 2}},
            {"event": "page_closed", "data": {"index": 0}},
            {"event": "page_closed", "data": {"index": 1}},
        ]
        self.assertEqual(self.probe.validate_scenario(rows, 2, 1), [])
