from __future__ import annotations

import os
import unittest
from typing import Any
from unittest.mock import patch

from moli_cdp_smoke.groups.multi_page_support import (
    multi_page_case_name,
    run_multi_page_cases,
    select_multi_page_cases,
)


async def _alpha(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    results.append({"case": "alpha"})


async def _beta(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    results.append({"case": "beta"})


async def _gamma(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    results.append({"case": "gamma"})


class MultiPageCaseSelectionTests(unittest.TestCase):
    def test_case_name_hides_private_function_prefix(self) -> None:
        self.assertEqual(multi_page_case_name(_alpha), "alpha")

    def test_explicit_selection_preserves_requested_order_and_deduplicates(
        self,
    ) -> None:
        selected = select_multi_page_cases(
            (_alpha, _beta, _gamma),
            requested="gamma, alpha,gamma",
            shard="",
        )

        self.assertEqual(selected, (_gamma, _alpha))

    def test_one_based_shards_partition_the_selected_cases(self) -> None:
        first = select_multi_page_cases(
            (_alpha, _beta, _gamma),
            requested="",
            shard="1/2",
        )
        second = select_multi_page_cases(
            (_alpha, _beta, _gamma),
            requested="",
            shard="2/2",
        )

        self.assertEqual(first, (_alpha, _gamma))
        self.assertEqual(second, (_beta,))

    def test_unknown_case_and_invalid_shard_are_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "unknown MOLI_MULTI_PAGE_CASES"):
            select_multi_page_cases((_alpha,), requested="missing", shard="")
        with self.assertRaisesRegex(ValueError, "one-based INDEX/COUNT"):
            select_multi_page_cases((_alpha,), requested="", shard="zero")
        with self.assertRaisesRegex(ValueError, "1 <= INDEX <= COUNT"):
            select_multi_page_cases((_alpha,), requested="", shard="2/1")


class MultiPageCaseRunnerTests(unittest.IsolatedAsyncioTestCase):
    async def test_runner_finishes_remaining_cases_before_reporting_failures(
        self,
    ) -> None:
        observed: list[str] = []

        async def _fails(
            browser: Any,
            fixture: str,
            results: list[dict[str, Any]],
        ) -> None:
            observed.append("fails")
            raise RuntimeError("expected failure")

        async def _survives(
            browser: Any,
            fixture: str,
            results: list[dict[str, Any]],
        ) -> None:
            observed.append("survives")

        with (
            patch.dict(
                os.environ,
                {"MOLI_MULTI_PAGE_CASES": "", "MOLI_MULTI_PAGE_SHARD": ""},
            ),
            self.assertRaises(ExceptionGroup) as raised,
        ):
            await run_multi_page_cases(
                object(),
                "http://fixture.test",
                [],
                (_fails, _survives),
                timeout_seconds=1,
            )

        self.assertEqual(observed, ["fails", "survives"])
        self.assertEqual(len(raised.exception.exceptions), 1)
        self.assertIn("expected failure", str(raised.exception.exceptions[0]))


if __name__ == "__main__":
    unittest.main()
