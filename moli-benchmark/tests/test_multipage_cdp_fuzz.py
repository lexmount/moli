import copy
import unittest
from unittest.mock import AsyncMock

from moli_benchmark.multipage_cdp_fuzz import Case, differences


class ExceptionComparisonTests(unittest.IsolatedAsyncioTestCase):
    async def test_distinguishes_uncaught_exception_types_without_stack_or_object_ids(self):
        case = object.__new__(Case)
        case.sessions = {}

        async def evaluate(name, object_id, stack):
            case.command = AsyncMock(return_value={"exceptionDetails": {
                "text": "Uncaught", "exception": {"className": name,
                    "objectId": object_id, "description": f"{name}: denied\n{stack}"},
            }})
            return await case.evaluate("probe()", "page")

        security = await evaluate("SecurityError", "1", "at first")
        type_error = await evaluate("TypeError", "2", "at second")
        self.assertTrue(differences(security, type_error))
        self.assertEqual(security, await evaluate("SecurityError", "3", "at another"))


def event(kind, visible, focused):
    return {"event": kind, "visibility": visible, "focus": focused}


class ActivityComparisonTests(unittest.TestCase):
    def test_accepts_both_observed_focus_gain_orders_without_mutating_evidence(self):
        first = [event("focus", "hidden", True), event("visibilitychange", "visible", True)]
        second = [event("visibilitychange", "visible", False), event("focus", "visible", True)]
        saved = copy.deepcopy([first, second])
        self.assertEqual(differences(first, second), [])
        self.assertEqual(differences(second, first), [])
        self.assertEqual([first, second], saved)

    def test_keeps_incorrect_intermediate_states_and_missing_events(self):
        valid = [event("visibilitychange", "visible", False), event("focus", "visible", True)]
        for invalid in [
            [event("visibilitychange", "visible", True), event("focus", "visible", True)],
            [event("focus", "hidden", True)],
            [event("focus", "hidden", False), event("visibilitychange", "visible", True)],
            valid + [event("focus", "visible", True)],
        ]:
            self.assertTrue(differences(valid, invalid), invalid)

    def test_accepts_both_observed_focus_loss_orders(self):
        first = [event("blur", "visible", False), event("visibilitychange", "hidden", False)]
        second = [event("visibilitychange", "hidden", False), event("blur", "hidden", False)]
        self.assertEqual(differences(first, second), [])

    def test_keeps_incorrect_focus_loss_states_and_missing_events(self):
        valid = [event("blur", "visible", False), event("visibilitychange", "hidden", False)]
        for invalid in [
            [event("blur", "hidden", True), event("visibilitychange", "hidden", False)],
            [event("visibilitychange", "hidden", False)],
            [event("blur", "visible", False)],
        ]:
            self.assertTrue(differences(valid, invalid), invalid)
