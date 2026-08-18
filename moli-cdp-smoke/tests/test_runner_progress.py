from __future__ import annotations

import asyncio
import io
import unittest
from contextlib import redirect_stderr

from moli_cdp_smoke.progress import await_with_progress


class RunnerProgressTests(unittest.IsolatedAsyncioTestCase):
    async def test_success_reports_start_and_done(self) -> None:
        stderr = io.StringIO()

        with redirect_stderr(stderr):
            result = await await_with_progress(
                "test/success",
                asyncio.sleep(0, result="done"),
            )

        self.assertEqual(result, "done")
        lines = stderr.getvalue().splitlines()
        self.assertEqual(lines[0], "[moli-cdp-smoke] START test/success")
        self.assertRegex(
            lines[1],
            r"^\[moli-cdp-smoke\] DONE test/success elapsed=\d+\.\d{3}s$",
        )

    async def test_failure_reports_error_type_before_reraising(self) -> None:
        async def fail() -> None:
            raise RuntimeError("expected failure")

        stderr = io.StringIO()
        with redirect_stderr(stderr), self.assertRaisesRegex(
            RuntimeError,
            "expected failure",
        ):
            await await_with_progress("test/failure", fail())

        lines = stderr.getvalue().splitlines()
        self.assertEqual(lines[0], "[moli-cdp-smoke] START test/failure")
        self.assertRegex(
            lines[1],
            r"^\[moli-cdp-smoke\] FAIL test/failure "
            r"elapsed=\d+\.\d{3}s error=RuntimeError$",
        )

    async def test_cancellation_is_reported_before_propagating(self) -> None:
        async def cancel() -> None:
            raise asyncio.CancelledError

        stderr = io.StringIO()
        with redirect_stderr(stderr), self.assertRaises(asyncio.CancelledError):
            await await_with_progress("test/cancel", cancel())

        lines = stderr.getvalue().splitlines()
        self.assertEqual(lines[0], "[moli-cdp-smoke] START test/cancel")
        self.assertRegex(
            lines[1],
            r"^\[moli-cdp-smoke\] FAIL test/cancel "
            r"elapsed=\d+\.\d{3}s error=CancelledError$",
        )


if __name__ == "__main__":
    unittest.main()
