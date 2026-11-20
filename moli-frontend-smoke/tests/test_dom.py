from __future__ import annotations

import pytest

from moli_frontend_smoke.dom import (
    dom_hash,
    first_difference,
    normalize_dom_node,
    unified_dom_diff,
)


def test_normalize_removes_ids_and_sorts_attributes() -> None:
    raw = {
        "nodeId": 19,
        "backendNodeId": 201,
        "nodeType": 1,
        "nodeName": "DIV",
        "localName": "div",
        "namespaceURI": "http://www.w3.org/1999/xhtml",
        "nodeValue": "",
        "attributes": ["z", "last", "a", "first"],
        "children": [
            {
                "nodeId": 20,
                "backendNodeId": 202,
                "nodeType": 3,
                "nodeName": "#text",
                "localName": "",
                "nodeValue": " exact whitespace ",
            }
        ],
    }
    assert normalize_dom_node(raw) == {
        "nodeType": 1,
        "nodeName": "DIV",
        "localName": "div",
        "namespaceURI": "http://www.w3.org/1999/xhtml",
        "nodeValue": "",
        "attributes": [["a", "first"], ["z", "last"]],
        "children": [
            {
                "nodeType": 3,
                "nodeName": "#text",
                "nodeValue": " exact whitespace ",
            }
        ],
    }


def test_normalize_preserves_labeled_subtrees() -> None:
    raw = {
        "nodeType": 1,
        "nodeName": "DIV",
        "localName": "div",
        "nodeValue": "",
        "shadowRoots": [
            {
                "nodeType": 11,
                "nodeName": "#document-fragment",
                "localName": "",
                "nodeValue": "",
                "shadowRootType": "user-agent",
            }
        ],
        "pseudoElements": [
            {
                "nodeType": 1,
                "nodeName": "::marker",
                "localName": "::marker",
                "nodeValue": "",
                "pseudoType": "marker",
            }
        ],
        "templateContent": {
            "nodeType": 11,
            "nodeName": "#document-fragment",
            "localName": "",
            "nodeValue": "",
        },
    }
    normalized = normalize_dom_node(raw)
    assert "shadowRoots" in normalized
    assert normalized["shadowRoots"][0]["shadowRootType"] == "user-agent"
    assert normalized["pseudoElements"][0]["pseudoType"] == "marker"
    assert "templateContent" in normalized


def _element_with_style(value: str) -> dict[str, object]:
    return {
        "nodeType": 1,
        "nodeName": "DIV",
        "localName": "div",
        "nodeValue": "",
        "attributes": ["style", value],
    }


def test_normalize_ignores_independent_style_declaration_order() -> None:
    chromium = normalize_dom_node(
        _element_with_style("color: rgb(12, 34, 56); margin-top: 7px;")
    )
    moli = normalize_dom_node(
        _element_with_style("margin-top: 7px; color: rgb(12, 34, 56);")
    )

    assert chromium == moli
    assert dom_hash(chromium) == dom_hash(moli)


def test_normalize_preserves_style_value_and_priority_differences() -> None:
    reference = normalize_dom_node(
        _element_with_style("color: red; margin-top: 7px;")
    )
    different_value = normalize_dom_node(
        _element_with_style("margin-top: 8px; color: red;")
    )
    different_priority = normalize_dom_node(
        _element_with_style("margin-top: 7px; color: red !important;")
    )

    assert reference != different_value
    assert reference != different_priority


def test_normalize_keeps_duplicate_property_order_strict() -> None:
    red_wins = normalize_dom_node(
        _element_with_style("color: blue; color: red; margin-top: 7px;")
    )
    blue_wins = normalize_dom_node(
        _element_with_style("color: red; color: blue; margin-top: 7px;")
    )

    assert red_wins != blue_wins


def test_normalize_keeps_shorthand_and_logical_order_strict() -> None:
    shorthand_first = normalize_dom_node(
        _element_with_style("margin: 0; margin-top: 7px;")
    )
    shorthand_last = normalize_dom_node(
        _element_with_style("margin-top: 7px; margin: 0;")
    )
    logical_first = normalize_dom_node(
        _element_with_style("margin-inline-start: 1px; margin-left: 2px;")
    )
    logical_last = normalize_dom_node(
        _element_with_style("margin-left: 2px; margin-inline-start: 1px;")
    )

    assert shorthand_first != shorthand_last
    assert logical_first != logical_last


def test_normalize_parses_nested_delimiters_without_splitting_declarations() -> None:
    first = normalize_dom_node(
        _element_with_style('content: "a;b:c"; color: rgb(1, 2, 3);')
    )
    second = normalize_dom_node(
        _element_with_style('color: rgb(1, 2, 3); content: "a;b:c";')
    )

    assert first == second


def test_normalize_keeps_invalid_or_comment_bearing_style_strict() -> None:
    invalid_first = normalize_dom_node(
        _element_with_style("color: red; broken; margin-top: 1px;")
    )
    invalid_last = normalize_dom_node(
        _element_with_style("margin-top: 1px; broken; color: red;")
    )
    comment_first = normalize_dom_node(
        _element_with_style("color: red; /* position */ margin-top: 1px;")
    )
    comment_last = normalize_dom_node(
        _element_with_style("margin-top: 1px; /* position */ color: red;")
    )

    assert invalid_first != invalid_last
    assert comment_first != comment_last


def test_difference_reports_first_json_path() -> None:
    left = {"nodeType": 1, "children": [{"nodeValue": "left"}]}
    right = {"nodeType": 1, "children": [{"nodeValue": "right"}]}
    assert first_difference(left, right) == "$.children[0].nodeValue"
    assert "-      \"nodeValue\": \"left\"" in unified_dom_diff(left, right)
    assert dom_hash(left) != dom_hash(right)


@pytest.mark.parametrize("logical,physical", [
    ("border-start-start-radius", "border-top-left-radius"),
    ("border-start-end-radius", "border-top-right-radius"),
    ("border-end-start-radius", "border-bottom-left-radius"),
    ("border-end-end-radius", "border-bottom-right-radius"),
])
def test_normalize_preserves_logical_corner_radius_cascade(logical: str, physical: str) -> None:
    # In horizontal-tb/LTR these declarations target the same corner; the last
    # declaration wins. Reversing them changes the resulting radius from 2px to 1px.
    first = normalize_dom_node(_element_with_style(f"{logical}: 1px; {physical}: 2px;"))
    second = normalize_dom_node(_element_with_style(f"{physical}: 2px; {logical}: 1px;"))
    assert first_difference(first, second) is not None
    assert dom_hash(first) != dom_hash(second)


def test_normalize_still_reorders_independent_physical_corners() -> None:
    first = normalize_dom_node(_element_with_style(
        "border-top-left-radius: 1px; border-bottom-right-radius: 2px;"
    ))
    second = normalize_dom_node(_element_with_style(
        "border-bottom-right-radius: 2px; border-top-left-radius: 1px;"
    ))
    assert dom_hash(first) == dom_hash(second)
