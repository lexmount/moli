from __future__ import annotations

import asyncio
import sys
import unittest
from typing import Any, cast

from moli_webdriver_smoke.serve import (
    MoliServe,
    _collect_process_output,
    render_moli_serve_diagnostics,
    stop_moli_serve,
)


class _ExitedProcess:
    returncode = 0


class MoliServeDiagnosticsTests(unittest.IsolatedAsyncioTestCase):
    async def test_collects_long_child_lines_and_final_unterminated_output(self) -> None:
        # Real subprocess pipes use StreamReader's default 64 KiB line limit.
        payload = "🙂" * 40000
        process = await asyncio.create_subprocess_exec(
            sys.executable,
            "-c",
            "import sys; sys.stdout.write('🙂' * 40000 + '\u005cnlast line')",
            stdout=asyncio.subprocess.PIPE,
        )
        logs: list[str] = []
        try:
            await asyncio.wait_for(
                _collect_process_output(process.stdout, logs, "stdout"), timeout=5
            )
            await asyncio.wait_for(process.wait(), timeout=5)
        finally:
            if process.returncode is None:
                process.kill()
                await process.wait()
        self.assertEqual(process.returncode, 0)
        self.assertEqual(logs, [f"stdout: {payload}", "stdout: last line"])

    async def test_collects_multiple_lines_in_one_chunk(self) -> None:
        stream = asyncio.StreamReader()
        stream.feed_data(b"first\n\nthird\r\n")
        stream.feed_eof()
        logs: list[str] = []
        await _collect_process_output(stream, logs, "stderr")
        self.assertEqual(logs, ["stderr: first", "stderr: ", "stderr: third"])

    async def test_stop_drains_late_child_output(self) -> None:
        logs: list[str] = []

        async def collect_late_output() -> None:
            await asyncio.sleep(0)
            logs.append("stderr: request handler panicked")

        collector = asyncio.create_task(collect_late_output())
        serve = MoliServe(
            process=cast(Any, _ExitedProcess()),
            logs=logs,
            tasks=[collector],
        )

        await stop_moli_serve(serve)

        self.assertFalse(collector.cancelled())
        self.assertEqual(logs, ["stderr: request handler panicked"])

    def test_diagnostics_are_bounded_to_the_requested_tail(self) -> None:
        serve = MoliServe(
            process=cast(Any, _ExitedProcess()),
            logs=["stdout: first", "stderr: second", "stderr: third"],
            tasks=[],
        )

        diagnostics = render_moli_serve_diagnostics(serve, max_lines=2)

        self.assertEqual(
            diagnostics,
            "moli serve diagnostics (last 2 of 3 captured lines):\n"
            "stderr: second\n"
            "stderr: third",
        )

    def test_empty_diagnostics_state_is_explicit(self) -> None:
        serve = MoliServe(
            process=cast(Any, _ExitedProcess()),
            logs=[],
            tasks=[],
        )

        self.assertIn("<no child output captured>", render_moli_serve_diagnostics(serve))


if __name__ == "__main__":
    unittest.main()
