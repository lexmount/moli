from __future__ import annotations

import asyncio
from collections import deque
from contextlib import redirect_stderr
from io import StringIO
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest

from moli_benchmark.wpt_cross.__main__ import _cdp_case_run, _run_case_path, main
from moli_benchmark.wpt_cross.case_set import enumerate_cases, explicit_case
from moli_benchmark.wpt_cross.runner import (
    CrashtestRun,
    _PageSessionUnusable,
    _case_parts,
    _case_test_type,
    _run_one_crashtest,
    case_result_to_dict,
)


URL = "http://localhost/dom/example-crash.html"
CASE = CrashtestRun("dom/example-crash.html", URL, 0.5)


def commit(url: str = URL, loader: str = "loader") -> dict:
    return {
        "method": "Page.frameNavigated",
        "sessionId": "session",
        "params": {"frame": {"id": "frame", "loaderId": loader, "url": url}},
    }


def value(payload: dict) -> dict:
    return {"result": {"result": {"value": payload}}}


class FakeClient:
    def __init__(self, phases: list[list[dict | BaseException]]):
        self.phases = deque(phases)
        self.pending: deque[dict | BaseException] = deque()
        self.commands: list[tuple[str, dict | None]] = []

    async def send(self, method: str, params: dict | None = None, *, session_id: str) -> int:
        self.commands.append((method, params))
        command_id = len(self.commands)
        for message in self.phases.popleft():
            if isinstance(message, dict) and "method" not in message:
                message = {**message, "id": command_id}
            self.pending.append(message)
        return command_id

    async def recv(self) -> dict:
        message = self.pending.popleft()
        if isinstance(message, BaseException):
            raise message
        return message


def phases() -> list[list[dict | BaseException]]:
    return [
        [{"result": {}}],
        [commit(), {"result": {"frameId": "frame", "loaderId": "loader"}}],
        [value({"href": URL})],
        [value({"complete": True})],
    ]


class CrashtestTests(unittest.TestCase):
    def run_case(self, replies: list[list[dict | BaseException]]):
        client = FakeClient(replies)
        try:
            result = asyncio.run(_run_one_crashtest(
                client=client, session_id="session", target_id="target", case=CASE,
            ))
        except _PageSessionUnusable as error:
            result = error.case_result
        return result, client

    def test_name_rules_distinguish_markup_from_script_harnesses(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            expected = {
                "dom/example-crash.html": "crashtest",
                "dom/example-crash.sub.html": "crashtest",
                "dom/crashtests/example.htm": "crashtest",
                "dom/crashtests/example.svg": "crashtest",
                "dom/crashtests/example.xhtml": "crashtest",
                "dom/crashtests/example.any.js": "testharness",
                "dom/crashtests/example.window.js": "testharness",
                "dom/crash.html": "testharness",
                "dom/example-crash-other.html": "testharness",
                "dom/example.html?mode=-crash": "testharness",
            }
            for name, test_type in expected.items():
                path = root / name.split("?", 1)[0]
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("<!doctype html>")
                with self.subTest(name=name):
                    self.assertEqual(explicit_case(root, name).test_type, test_type)

    def test_discovery_applies_support_filters_and_crashtest_precedence(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for name, source in {
                "dom/example-crash.html": '<meta name="variant" content="?ignored">',
                "dom/crashtests/also-harness.html": '<script src="/resources/testharness.js"></script>',
                "dom/ordinary.html": "<!doctype html>",
                "dom/resources/support-crash.html": "<!doctype html>",
                "css/css-flexbox/layout-crash.html": "<!doctype html>",
            }.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(source)
            cases = enumerate_cases(root)
            self.assertEqual([(case.case_path, case.test_type) for case in cases], [
                ("dom/crashtests/also-harness.html", "crashtest"),
                ("dom/example-crash.html", "crashtest"),
            ])

    def test_run_metadata_and_cli_mode_do_not_treat_crashtests_as_harnesses(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "example-crash.html").write_text("<!doctype html>")
            case = explicit_case(root, "example-crash.html")
            server = SimpleNamespace(url_for_case=lambda path, **_: "http://localhost/" + path)
            run = _cdp_case_run(server, case, external=False, timeout_seconds=9)
            self.assertIsInstance(run, CrashtestRun)
            self.assertEqual(_case_parts(run, 1), (case.case_path, "http://localhost/example-crash.html", 9))
            self.assertEqual(_case_test_type(run), "crashtest")
            self.assertEqual(_run_case_path(run), case.case_path)
            errors = StringIO()
            with redirect_stderr(errors):
                code = main(["--wpt-root", str(root), "--engine", "moli", "--mode", "cli",
                             "--case", case.case_path, "--output-dir", str(root / "out")])
            self.assertEqual(code, 4)
            self.assertIn("crashtests require CDP", errors.getvalue())

    def test_completion_reports_no_invented_subtests(self):
        result, client = self.run_case(phases())
        self.assertEqual(result.status, "pass")
        self.assertEqual(result.test_type, "crashtest")
        self.assertEqual(result.payload_source, "crashtest-ready")
        self.assertEqual(case_result_to_dict(result)["subtests"]["total"], 0)
        self.assertTrue(client.commands[-1][1]["awaitPromise"])

    def test_waiting_page_times_out_without_a_completion_signal(self):
        replies = phases()
        replies[-1] = [asyncio.TimeoutError()]
        result, _ = self.run_case(replies)
        self.assertEqual(result.status, "timeout")
        self.assertIsNone(result.payload_source)

    def test_renderer_crash_is_observed_before_command_response(self):
        for crash in [
            {"method": "Inspector.targetCrashed", "sessionId": "session"},
            {"method": "Target.targetCrashed", "params": {"targetId": "target"}},
        ]:
            with self.subTest(crash=crash):
                replies = phases()
                replies[-1] = [crash]
                result, _ = self.run_case(replies)
                self.assertEqual(result.status, "crash")

    def test_other_targets_crashing_do_not_impersonate_this_target(self):
        replies = phases()
        replies[-1][:0] = [
            {"method": "Inspector.targetCrashed", "sessionId": "other"},
            {"method": "Target.targetCrashed", "params": {"targetId": "other"}},
        ]
        result, _ = self.run_case(replies)
        self.assertEqual(result.status, "pass")

    def test_wrong_navigation_cannot_report_pass(self):
        replies = phases()
        replies[1][0] = commit("http://localhost/other.html")
        result, client = self.run_case(replies)
        self.assertEqual(result.status, "error")
        self.assertIn("unexpected URL", result.error)
        self.assertEqual(len(client.commands), 3)

    def test_stale_loader_is_ignored_before_wait_script_runs(self):
        replies = phases()
        replies[1][0] = commit(loader="previous-loader")
        replies.insert(3, [commit(), value({"href": URL})])
        result, client = self.run_case(replies)
        self.assertEqual(result.status, "pass")
        self.assertEqual(len(client.commands), 5)

    def test_script_and_protocol_errors_are_not_completion(self):
        for reply in [
            {"result": {"exceptionDetails": {"text": "readiness failed"}}},
            {"error": {"message": "session closed"}},
            value({"complete": False}),
        ]:
            with self.subTest(reply=reply):
                replies = phases()
                replies[-1] = [reply]
                result, _ = self.run_case(replies)
                self.assertEqual(result.status, "error")
                self.assertIsNone(result.payload_source)


if __name__ == "__main__":
    unittest.main()
