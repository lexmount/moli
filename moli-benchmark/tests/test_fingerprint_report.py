from copy import deepcopy
import json
from pathlib import Path
import tempfile
import unittest

from moli_benchmark.fingerprint.report import compare, load_summary, metrics, render_html, write_report


def summary():
    return {"schema_version": 1, "matrix_complete": True, "inputs_unchanged": True,
            "manifest": {"revision": "abc", "workload_hash": "fixed", "environment": {"platform": "test"}},
            "suites": [{"suite": "fingerprint", "coverage": {"moli": {"complete": 1}}, "samples": [
                {"engine": "moli", "site": "device-browser-static", "run": 1, "status": "complete",
                 "report": {"valid": True, "isBot": False, "details": {"isAutomatedWithCDP": False}},
                 "browser": {"product": "Moli"}}]}]}


class FingerprintReportTests(unittest.TestCase):
    def test_identical_report_has_no_changes(self):
        result = compare(summary(), summary())
        self.assertTrue(result["comparable_workload"])
        self.assertEqual(result["changes"], [])

    def test_negative_site_verdict_and_missing_are_not_hidden(self):
        new = summary()
        row = new["suites"][0]["samples"][0]
        row["report"]["isBot"] = True
        del row["report"]["details"]
        result = compare(new, summary())
        changes = {item["field"]: item for item in result["changes"][0]["fields"]}
        self.assertEqual(changes["isBot"]["old"], False)
        self.assertEqual(changes["isBot"]["new"], True)
        self.assertNotIn("delta", changes["isBot"])
        self.assertFalse(changes["details.isAutomatedWithCDP"]["new_present"])
        self.assertNotIn("pass_count", result)

    def test_false_zero_null_and_absence_are_different(self):
        old = summary()
        new = deepcopy(old)
        new["suites"][0]["samples"][0]["report"]["isBot"] = 0
        changes = compare(new, old)["changes"][0]["fields"]
        self.assertEqual(len(changes), 1)
        self.assertNotIn("delta", changes[0])
        new["suites"][0]["samples"][0]["report"]["extra"] = None
        self.assertTrue(any(item["field"] == "extra" and not item["old_present"]
                            for item in compare(new, old)["changes"][0]["fields"]))

    def test_numeric_changes_are_not_automatically_called_regressions(self):
        old, new = summary(), summary()
        for report, score in [(old, 12), (new, 8)]:
            row = report["suites"][0]["samples"][0]
            row.update(site="fingerprint-pro", report={"valid": True, "results": [{"suspect_score": score}]})
        result = compare(new, old)
        self.assertEqual(result["changes"][0]["fields"][0]["delta"], -4)
        self.assertNotIn("regression", result)

    def test_mismatched_workloads_or_incomplete_runs_have_caveats(self):
        new = summary()
        new["manifest"]["workload_hash"] = "changed"
        new["matrix_complete"] = False
        new["inputs_unchanged"] = False
        result = compare(new, summary())
        self.assertFalse(result["comparable_workload"])
        self.assertEqual(len(result["caveats"]), 3)

    def test_added_removed_samples_and_browser_script_changes_are_visible(self):
        new = summary()
        new["suites"][0]["samples"][0]["run"] = 2
        result = compare(new, summary())
        self.assertEqual(len(result["changes"]), 2)
        self.assertEqual(result["changes"][0]["new_status"], "not_sampled")
        old = summary()
        new = deepcopy(old)
        new["suites"][0]["samples"][0].update(browser={"product": "new"}, observations={"scripts": [{"sha256": "abc"}]})
        change = compare(new, old)["changes"][0]
        self.assertTrue(change["script_set_changed"])
        self.assertTrue(change["browser_changed"])

    def test_html_escapes_untrusted_values_and_shows_coverage(self):
        value = summary()
        value["suites"][0]["samples"][0]["report"]["payload"] = '<script>alert("x")</script>'
        html = render_html(value)
        self.assertNotIn("<script>", html)
        self.assertIn("&lt;script&gt;", html)
        self.assertIn("Collection completeness is not a bot pass rate", html)
        self.assertIn("isBot", html)

    def test_read_write_and_validation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "summary.json").write_text(json.dumps(summary()))
            self.assertEqual(load_summary(root), summary())
            write_report(root, summary(), summary())
            self.assertTrue((root / "index.html").is_file())
            self.assertEqual(json.loads((root / "comparison.json").read_text())["changes"], [])
            for malformed in [{}, {"schema_version": 2}, {"schema_version": 1, "suites": [{"suite": "startup"}]}]:
                (root / "summary.json").write_text(json.dumps(malformed))
                with self.assertRaises(ValueError):
                    load_summary(root)
            duplicate = summary()
            duplicate["suites"][0]["samples"] *= 2
            (root / "summary.json").write_text(json.dumps(duplicate))
            with self.assertRaises(ValueError):
                load_summary(root)


if __name__ == "__main__":
    unittest.main()
