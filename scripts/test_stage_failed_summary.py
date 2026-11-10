"""Run: PYTHONPATH=moli-benchmark python -m unittest discover -s scripts -p test_stage_failed_summary.py"""
from __future__ import annotations

import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch


class FailedStageSummaryTests(unittest.TestCase):
    def test_failed_runs_keep_error_details_in_summary(self) -> None:
        script = Path(__file__).with_name("run-detailed-stage-compare.py")
        spec = importlib.util.spec_from_file_location("stage_compare", script)
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        cases = [
            {"run": 1, "elapsed_ms": 10, "error": "DCL timed out"},
            {"run": 2, "elapsed_ms": 20, "timings": {"error": "startup timeout: refused"}},
            {"run": 3, "elapsed_ms": 30, "error": str(TimeoutError())},
            {"run": 4, "elapsed_ms": 40, "timings": {"error": ""}},
            {"run": 5, "elapsed_ms": 50, "timings": {
                "create_target_ms": 1, "attach_target_ms": 2, "enable_domains_ms": 3,
                "navigate_ack_ms": 4, "navigate_to_dcl_ms": 5, "outer_html_ms": 6, "html_length": 7,
            }},
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "target/release/moli"
            binary.parent.mkdir(parents=True)
            binary.touch()
            output = io.StringIO()
            with patch.object(module, "REPO_ROOT", root), patch.object(module, "run_single_test", side_effect=lambda **kwargs: [dict(case, url=kwargs["url"]) for case in cases]), redirect_stdout(output):
                module.main()
            text = output.getvalue()
            self.assertIn("DCL timed out", text)
            self.assertIn("startup timeout: refused", text)
            self.assertIn("outer_html: 6.0ms", text)
            self.assertEqual(text.count("Error: <no error details>"), 4)
            saved = list((root / "tmp").glob("detailed-stage-compare-*.json"))
            self.assertEqual(len(saved), 1)
            self.assertEqual(len(json.loads(saved[0].read_text())), 10)


if __name__ == "__main__":
    unittest.main()
