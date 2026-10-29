from __future__ import annotations

import unittest

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.groups.dom_publication import _DomMirror


def node(node_id, **fields):
    return {"nodeId": node_id, "nodeType": 1, **fields}


class DomPublicationMirrorTests(unittest.TestCase):
    def test_description_does_not_publish_a_node(self):
        mirror = _DomMirror()
        mirror.describe(node(0, backendNodeId=123))
        with self.assertRaisesRegex(SmokeError, "unpublished"):
            mirror.require(123, "getSearchResults response")

    def test_result_must_be_known_before_a_later_notification(self):
        mirror = _DomMirror()
        mirror.remember(node(1))
        with self.assertRaisesRegex(SmokeError, "unpublished"):
            mirror.require(2, "getSearchResults response")
        mirror.observe({"method": "DOM.setChildNodes", "params": {
            "parentId": 1, "nodes": [node(2, parentId=1)],
        }})
        mirror.require(2, "getSearchResults response")

    def test_unknown_event_targets_and_parents_fail(self):
        for method, params in [
            ("DOM.attributeModified", {"nodeId": 2}),
            ("DOM.attributeRemoved", {"nodeId": 2}),
            ("DOM.characterDataModified", {"nodeId": 2}),
            ("DOM.childNodeCountUpdated", {"nodeId": 2}),
            ("DOM.setChildNodes", {"parentId": 2, "nodes": []}),
            ("DOM.childNodeInserted", {"parentNodeId": 1, "previousNodeId": 2, "node": node(3)}),
        ]:
            with self.subTest(method=method):
                mirror = _DomMirror()
                mirror.remember(node(1))
                with self.assertRaisesRegex(SmokeError, "unpublished"):
                    mirror.observe({"method": method, "params": params})

    def test_replacement_and_removal_retire_all_associated_descendants(self):
        for update in [
            {"method": "DOM.setChildNodes", "params": {"parentId": 1, "nodes": [node(2)]}},
            {"method": "DOM.childNodeRemoved", "params": {"parentNodeId": 1, "nodeId": 2}},
            {"method": "DOM.documentUpdated", "params": {}},
        ]:
            with self.subTest(update=update):
                mirror = _DomMirror()
                mirror.remember(node(1, children=[node(2, children=[node(3)],
                    shadowRoots=[node(4, children=[node(5)])],
                    contentDocument=node(6, children=[node(7)]),
                    templateContent=node(8), pseudoElements=[node(9)])]))
                mirror.observe(update)
                for node_id in range(3, 10):
                    with self.assertRaisesRegex(SmokeError, "unpublished"):
                        mirror.require(node_id, "retired descendant")


if __name__ == "__main__":
    unittest.main()
