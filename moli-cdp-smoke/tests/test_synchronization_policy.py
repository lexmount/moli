"""Make new timing assumptions visible in review instead of silently copying them."""

from __future__ import annotations

import ast
from pathlib import Path
import unittest


PACKAGE = Path(__file__).resolve().parents[1] / "moli_cdp_smoke"


class SynchronizationPolicyTests(unittest.TestCase):
    def test_fixed_waits_require_a_documented_timing_contract(self) -> None:
        violations = []
        for path in sorted((PACKAGE / "groups").glob("*.py")):
            lines = path.read_text().splitlines()
            for call in ast.walk(ast.parse(path.read_text())):
                if not isinstance(call, ast.Call) or not isinstance(call.func, ast.Attribute):
                    continue
                fixed_wait = call.func.attr == "wait_for_timeout" or (
                    call.func.attr == "sleep" and call.args
                    and isinstance(call.args[0], ast.Constant)
                    and isinstance(call.args[0].value, (int, float)) and call.args[0].value > 0
                )
                if fixed_wait and "# smoke-timing:" not in lines[call.lineno - 2]:
                    violations.append(f"{path.name}:{call.lineno}: {ast.unparse(call)}")
        self.assertEqual(violations, [], "Use an identity/state/event barrier, or explain the actual timing contract")

    def test_readiness_probes_do_not_turn_arbitrary_errors_into_false(self) -> None:
        violations = []
        for path in sorted((PACKAGE / "groups").glob("*.py")):
            for handler in ast.walk(ast.parse(path.read_text())):
                if not (isinstance(handler, ast.ExceptHandler) and isinstance(handler.type, ast.Name)
                        and handler.type.id in {"Exception", "BaseException"}):
                    continue
                if any(isinstance(node, ast.Return) and isinstance(node.value, ast.Constant)
                       and node.value.value is False for node in ast.walk(handler)):
                    violations.append(f"{path.name}:{handler.lineno}")
        self.assertEqual(violations, [], "Protocol, fixture, and evaluation errors must remain failures")

    def test_python_cdp_sessions_use_the_observed_factory(self) -> None:
        violations = []
        for path in sorted(PACKAGE.rglob("*.py")):
            if path.name == "diagnostics.py":
                continue
            for call in ast.walk(ast.parse(path.read_text())):
                if (isinstance(call, ast.Call) and isinstance(call.func, ast.Attribute)
                        and call.func.attr == "new_cdp_session"):
                    violations.append(f"{path.relative_to(PACKAGE)}:{call.lineno}")
        self.assertEqual(violations, [], "Use diagnostics.new_cdp_session so failures retain protocol evidence")
