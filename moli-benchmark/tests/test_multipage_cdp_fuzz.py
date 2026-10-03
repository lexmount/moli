import copy
import unittest

from moli_benchmark.multipage_cdp_fuzz import differences


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
