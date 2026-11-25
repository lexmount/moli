from __future__ import annotations

import difflib
import hashlib
import json
from typing import Any, Iterator

from tinycss2 import parse_blocks_contents, serialize


NODE_FIELDS = ("nodeType", "nodeName", "localName", "namespaceURI", "nodeValue")
SCALAR_OPTIONAL_FIELDS = (
    "publicId",
    "systemId",
    "shadowRootType",
    "pseudoType",
)
EDGE_FIELDS = (
    "children",
    "shadowRoots",
    "pseudoElements",
    "templateContent",
    "contentDocument",
)

# Reordering a shorthand with one of its longhands, or two declarations for the
# same property, can change the cascade. Keep those blocks byte-sensitive. The
# comparison only canonicalizes independently-cascaded declarations.
_ORDER_SENSITIVE_STYLE_PROPERTIES = frozenset(
    {
        "all",
        "animation",
        "animation-range",
        "background",
        "background-position",
        "border",
        "border-block",
        "border-block-color",
        "border-block-style",
        "border-block-width",
        "border-bottom",
        "border-color",
        "border-image",
        "border-inline",
        "border-inline-color",
        "border-inline-style",
        "border-inline-width",
        "border-left",
        "border-radius",
        "border-start-start-radius",
        "border-start-end-radius",
        "border-end-start-radius",
        "border-end-end-radius",
        "border-right",
        "border-style",
        "border-top",
        "border-width",
        "column-rule",
        "columns",
        "contain-intrinsic-size",
        "container",
        "cue",
        "flex",
        "flex-flow",
        "font",
        "font-synthesis",
        "font-variant",
        "gap",
        "grid",
        "grid-area",
        "grid-column",
        "grid-column-gap",
        "grid-gap",
        "grid-row",
        "grid-row-gap",
        "grid-template",
        "inset",
        "inset-block",
        "inset-inline",
        "line-clamp",
        "list-style",
        "margin",
        "margin-block",
        "margin-inline",
        "marker",
        "mask",
        "mask-border",
        "offset",
        "outline",
        "overflow",
        "overscroll-behavior",
        "padding",
        "padding-block",
        "padding-inline",
        "page-break-after",
        "page-break-before",
        "page-break-inside",
        "pause",
        "place-content",
        "place-items",
        "place-self",
        "position-try",
        "rest",
        "scroll-margin",
        "scroll-margin-block",
        "scroll-margin-inline",
        "scroll-padding",
        "scroll-padding-block",
        "scroll-padding-inline",
        "scroll-timeline",
        "text-decoration",
        "text-emphasis",
        "text-wrap",
        "transition",
        "view-timeline",
        "white-space",
        "word-wrap",
    }
)
_STYLE_ORDER_CANONICAL_PREFIX = "@moli-style-order-v1:"


def _contains_css_parse_error(values: list[Any]) -> bool:
    for value in values:
        if getattr(value, "type", None) == "error":
            return True
        for field in ("content", "arguments"):
            nested = getattr(value, field, None)
            if isinstance(nested, list) and _contains_css_parse_error(nested):
                return True
    return False


def _style_declarations_can_be_reordered(names: list[str]) -> bool:
    if len(set(names)) != len(names):
        return False
    for name in names:
        if name.startswith("--"):
            continue
        if (
            name.startswith("-")
            or name in _ORDER_SENSITIVE_STYLE_PROPERTIES
            or "block" in name.split("-")
            or "inline" in name.split("-")
        ):
            return False
    return True


def _canonicalize_style_declaration_order(value: str) -> str:
    parsed = parse_blocks_contents(
        value,
        skip_comments=False,
        skip_whitespace=True,
    )
    declarations: list[tuple[str, str, bool]] = []
    for item in parsed:
        if getattr(item, "type", None) != "declaration":
            return value
        item_value = getattr(item, "value", None)
        if not isinstance(item_value, list) or _contains_css_parse_error(item_value):
            return value
        original_name = str(item.name)
        name = original_name if original_name.startswith("--") else str(item.lower_name)
        declarations.append((name, serialize(item_value).strip(), bool(item.important)))
    if len(declarations) < 2 or not _style_declarations_can_be_reordered(
        [name for name, _value, _important in declarations]
    ):
        return value
    declarations.sort()
    return _STYLE_ORDER_CANONICAL_PREFIX + json.dumps(
        declarations,
        ensure_ascii=False,
        separators=(",", ":"),
    )


def _attributes(value: Any) -> list[list[str]]:
    if not isinstance(value, list):
        return []
    pairs = []
    for index in range(0, len(value) - 1, 2):
        name = str(value[index])
        attribute_value = str(value[index + 1])
        if name == "style":
            attribute_value = _canonicalize_style_declaration_order(attribute_value)
        pairs.append([name, attribute_value])
    pairs.sort(key=lambda pair: (pair[0], pair[1]))
    return pairs


def normalize_dom_node(node: dict[str, Any]) -> dict[str, Any]:
    normalized: dict[str, Any] = {}
    for field in NODE_FIELDS:
        value = node.get(field)
        if value not in (None, "") or field in {"nodeType", "nodeName", "nodeValue"}:
            normalized[field] = value if value is not None else ""
    attributes = _attributes(node.get("attributes"))
    if attributes:
        normalized["attributes"] = attributes
    for field in SCALAR_OPTIONAL_FIELDS:
        value = node.get(field)
        if value not in (None, ""):
            normalized[field] = value
    for edge in EDGE_FIELDS:
        value = node.get(edge)
        if isinstance(value, list):
            if value:
                normalized[edge] = [
                    normalize_dom_node(child) for child in value if isinstance(child, dict)
                ]
        elif isinstance(value, dict):
            normalized[edge] = normalize_dom_node(value)
    return normalized


def iter_nodes(node: dict[str, Any]) -> Iterator[dict[str, Any]]:
    yield node
    for edge in EDGE_FIELDS:
        value = node.get(edge)
        if isinstance(value, list):
            for child in value:
                if isinstance(child, dict):
                    yield from iter_nodes(child)
        elif isinstance(value, dict):
            yield from iter_nodes(value)


def dom_hash(node: dict[str, Any]) -> str:
    encoded = json.dumps(node, sort_keys=True, ensure_ascii=False, separators=(",", ":"))
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def first_difference(left: Any, right: Any, path: str = "$") -> str | None:
    if type(left) is not type(right):
        return path
    if isinstance(left, dict):
        left_keys = list(left.keys())
        right_keys = list(right.keys())
        if left_keys != right_keys:
            first_key = next(
                (
                    key
                    for key in [*left_keys, *right_keys]
                    if (key in left) != (key in right)
                ),
                None,
            )
            return f"{path}.{first_key}" if first_key else path
        for key in left_keys:
            difference = first_difference(left[key], right[key], f"{path}.{key}")
            if difference:
                return difference
        return None
    if isinstance(left, list):
        if len(left) != len(right):
            return f"{path}.length"
        for index, (left_value, right_value) in enumerate(zip(left, right, strict=True)):
            difference = first_difference(left_value, right_value, f"{path}[{index}]")
            if difference:
                return difference
        return None
    return None if left == right else path


def unified_dom_diff(left: dict[str, Any], right: dict[str, Any]) -> str:
    left_lines = json.dumps(left, indent=2, ensure_ascii=False, sort_keys=False).splitlines()
    right_lines = json.dumps(right, indent=2, ensure_ascii=False, sort_keys=False).splitlines()
    return "\n".join(
        difflib.unified_diff(
            left_lines,
            right_lines,
            fromfile="chromium.dom.json",
            tofile="moli.dom.json",
            lineterm="",
        )
    )
