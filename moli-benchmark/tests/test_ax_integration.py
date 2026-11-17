from __future__ import annotations

import unittest
from pathlib import Path
import tempfile

from moli_benchmark.ax_cases import AX_CASES, Probe
from moli_benchmark.ax_integration import build_summary, compare_observations, expectation_errors, facts, tree_signature, write_report


class AxIntegrationTests(unittest.TestCase):
    def test_twenty_unique_documents_with_independent_expectations(self) -> None:
        self.assertEqual(len(AX_CASES), 20)
        self.assertEqual(len({case.name for case in AX_CASES}), 20)
        for case in AX_CASES:
            self.assertIn(f'data-benchmark-case="{case.name}"', case.html())
            self.assertIn('ready.setAttribute("data-benchmark-status","ok")', case.html())
            for step in case.steps:
                self.assertTrue(step.probes)
                self.assertFalse(step.probes[0].expected["ignored"])
                self.assertEqual(len({probe.id for probe in step.probes}), len(step.probes))

    def test_projection_keeps_semantics_and_only_normalizes_origin(self) -> None:
        node = {"ignored": False, "nodeId": "unstable", "backendDOMNodeId": 7,
                "role": {"value": "link"}, "name": {"value": "Link"},
                "properties": [{"name": "url", "value": {"value": "http://127.0.0.1:99/%C2%A0"}}]}
        actual = facts(node, "http://127.0.0.1:99")
        self.assertEqual(actual["url"], "{origin}/%C2%A0")
        self.assertEqual(actual["role"], "link")
        self.assertNotIn("nodeId", actual)
        self.assertFalse(actual["disabled"])
        other = {**node, "properties": [{"name": "url", "value": {"value": "http://127.0.0.1:990/wrong"}}]}
        self.assertEqual(facts(other, "http://127.0.0.1:99")["url"], "http://127.0.0.1:990/wrong")

    def test_missing_nodes_and_role_url_errors_cannot_pass(self) -> None:
        probes = (Probe("a", {"role": "link", "url": "{origin}/go"}),)
        self.assertTrue(expectation_errors(probes, {}))
        errors = expectation_errors(probes, {"a": {"role": "button", "url": None}})
        self.assertEqual({error["field"] for error in errors}, {"role", "url"})
        self.assertTrue(compare_observations({"a": {"role": "link"}}, {}))

    def test_missing_both_engines_is_failure_not_a_match(self) -> None:
        summary = build_summary({"chrome-cdp": {"rows": []}, "moli-cdp": {"rows": []}})
        self.assertEqual(summary["cases"], 20)
        self.assertEqual(summary["phases"], 22)
        self.assertEqual(summary["matching_phases"], 0)
        self.assertTrue(all(row["missing_target"] for row in summary["comparisons"]))

    def test_warm_tree_check_covers_unprobed_children_and_their_order(self) -> None:
        root = {"nodeId": "1", "role": {"value": "navigation"}, "childIds": ["2", "3"]}
        child = {"nodeId": "2", "role": {"value": "link"}, "name": {"value": "Original"}}
        original = {"nodes": [root, child]}
        self.assertEqual(tree_signature(original, "http://fixture"),
                         tree_signature({"nodes": [child, root]}, "http://fixture"))
        renamed = {"nodes": [root, {**child, "name": {"value": "Unexpected"}}]}
        reordered = {"nodes": [{**root, "childIds": ["3", "2"]}, child]}
        self.assertNotEqual(tree_signature(original, "http://fixture"), tree_signature(renamed, "http://fixture"))
        self.assertNotEqual(tree_signature(original, "http://fixture"), tree_signature(reordered, "http://fixture"))

    def test_comparison_only_failure_is_visible_in_report(self) -> None:
        row = {"case": "svg", "step": "initial", "errors": []}
        reports = {target: {"rows": [row]} for target in ["chrome-cdp", "moli-cdp"]}
        summary = {"cases": 1, "phases": 1, "matching_phases": 0, "passing_cases": [],
                   "expectation_failures": {"chrome-cdp": 0, "moli-cdp": 0},
                   "comparisons": [{"case": "svg", "step": "initial", "missing_target": False,
                                    "differences": {"a": {"chrome": {"focusable": True}, "moli": {"focusable": False}}}}]}
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            write_report(output, reports, summary)
            text = (output / "report.md").read_text()
            self.assertIn("FAIL", text)
            self.assertIn("focusable", text)


if __name__ == "__main__":
    unittest.main()
