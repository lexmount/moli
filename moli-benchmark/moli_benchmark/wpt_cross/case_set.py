"""Enumerate WPT cases that the cross-engine runner can execute.

The default semantic profile selects testharness cases. Layout profiles add a
deterministic static subset and read upstream ``MANIFEST.json`` for reftest
URLs, ``==`` / ``!=`` references, timeout metadata, and fuzzy bounds.

A testharness case is selectable if:

* The file does not live under one of the default excluded directory prefixes.
* It is a navigable ``.html`` file, a selected ``.any.js`` global, or a
  ``.window.js`` / ``.worker.js`` script case that can be wrapped as a
  testharness page.
* Its filename does not contain ``.tentative`` or ``.optional``.
* Its filename does not end in ``-manual.html``.
* It is not under a ``resources`` support directory.
* HTML cases reference ``/resources/testharness.js``.
* The file body does NOT reference ``/resources/testdriver`` (testdriver
  requires a WebDriver-style automation backend the v1 fixture server does
  not provide).
* No sibling ``.py`` file with the same stem exists (would need wptserve
  Python handler).
* The file body and directly referenced relative scripts do NOT reference
  unsupported wptserve ``.py`` handlers.
* The broad default file path does not contain ``.https.``, ``.h2.``,
  ``.sub.``, or ``.serviceworker.`` substrings unless the case is in the
  default Service Worker allowlist. Focused ``--dir-prefix`` runs may opt into
  ``.https.``, ``.sub.``, and ``.serviceworker.`` cases because the fixture
  server has local HTTP substitutions for those WPT features.

The default set is blacklist-based: start from all upstream WPT HTML
testharness cases the v1 static fixture server can serve, then drop areas
that require a real layout, paint, compositor, media timeline, or canvas
rasterizer. This keeps the baseline broad without letting rendering-only gaps
dominate the signal.
Focused directory runs can opt into tentative cases and additional ``.any.js``
wrapper variants explicitly. Focused runs also include ``.window.js`` /
``.worker.js`` script wrappers. The broad default keeps script wrappers out
except for semantic suites listed in ``DEFAULT_SCRIPT_CASE_DIR_PREFIXES``;
Streams is included because most upstream Streams coverage is authored as
script WPT rather than navigable HTML. Layout reftests additionally exclude
dynamic wptserve Python handlers, HTTP/2 cases, animation directories, and
media/canvas documents during the initial static baseline.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from html import unescape
from html.parser import HTMLParser
from pathlib import Path
from typing import Any, Iterator
from urllib.parse import urljoin, urlsplit

from .any_js import (
    ANY_JS_DEDICATED_WORKER_GLOBAL,
    ANY_JS_WINDOW_GLOBAL,
    any_js_case_path_for_global,
    is_any_js_case_path,
    is_script_js_case_path,
    normalize_any_js_window_case_path,
    normalize_script_js_case_path,
)

LONG_TIMEOUT_MULTIPLIER = 5.0
ANY_JS_WINDOW_QUERY_NAME = "__wpt_cross_any"
ANY_JS_WINDOW_QUERY_VALUE = "window"
ANY_JS_WINDOW_QUERY = f"{ANY_JS_WINDOW_QUERY_NAME}={ANY_JS_WINDOW_QUERY_VALUE}"
WINDOW_JS_WINDOW_QUERY_NAME = "__wpt_cross_window"
WINDOW_JS_WINDOW_QUERY_VALUE = "window"
WINDOW_JS_WINDOW_QUERY = f"{WINDOW_JS_WINDOW_QUERY_NAME}={WINDOW_JS_WINDOW_QUERY_VALUE}"


# Default blacklist prefixes (relative to wpt root). Keep this list scoped to
# layout, paint, compositor, media, and canvas-heavy areas so the default
# WPT-cross baseline stays a broad Web API sweep instead of a curated allowlist.
DEFAULT_EXCLUDE_DIR_PREFIXES: tuple[str, ...] = (
    "html/canvas",
    "html/rendering",
    "css/CSS2",
    "css/css-align",
    "css/css-anchor-position",
    "css/css-backgrounds",
    "css/css-borders",
    "css/css-box",
    "css/css-break",
    "css/css-contain",
    "css/css-display",
    "css/css-flexbox",
    "css/css-fonts",
    "css/css-gaps",
    "css/css-grid",
    "css/css-images",
    "css/css-inline",
    "css/css-lists",
    "css/css-logical",
    "css/css-masking",
    "css/css-multicol",
    "css/css-overflow",
    "css/css-page",
    "css/css-position",
    "css/css-pseudo",
    "css/css-ruby",
    "css/css-scroll-anchoring",
    "css/css-scroll-snap",
    "css/css-shapes",
    "css/css-sizing",
    "css/css-tables",
    "css/css-text",
    "css/css-transforms",
    "css/css-ui",
    "css/css-view-transitions",
    "css/css-writing-modes",
    "css/cssom-view",
    "css/css-exclusions",
    "css/css-paint-api",
    "css/filter-effects",
    "css/motion",
    "css/view-timeline",
    "css/visual-formatting-model",
    "scroll-animations",
    "layout-instability",
    "paint-timing",
    "largest-contentful-paint",
    "element-timing",
    "container-timing",
    "resize-observer",
    "intersection-observer",
    "visual-viewport",
    "svg/animations",
    "svg/painting",
    "svg/pservers",
    "svg/render",
    "svg/sizing",
    "svg/text",
    # Animation, media, capture, and realtime stacks need engines Moli
    # does not currently model in WPT-cross. Keep them available through
    # --dir-prefix, but exclude them from the default semantic baseline.
    "web-animations",
    "webaudio",
    "webvtt",
    "media-source",
    "mediacapture-record",
    "mediacapture-fromelement",
    "html-media-capture",
    "remote-playback",
    "mediasession",
    "webrtc",
    "webrtc-encoded-transform",
    "webrtc-extensions",
    "webrtc-svc",
    "webrtc-stats",
    "websockets",
    "webgl",
    "imagebitmap-renderingcontext",
    # JPEG XL conformance requires an actual codec and includes pixel-level
    # Canvas sampling. Moli only models lightweight image metadata and
    # load observability today, so keep codec fidelity out of the default
    # semantic baseline while preserving focused --dir-prefix runs.
    "jpegxl",
    "mediacapture-image",
    "mediacapture-streams",
    "mst-content-hint",
    "video-rvfc",
    # Accessibility/manual suites require an accessibility tree or manual
    # harness behavior outside this runner's current contract.
    "accname",
    "core-aam",
    "dpub-aam",
    "graphics-aam",
    "wai-aria",
    # MathML and forced-colors are mostly rendering/presentation signal here.
    "mathml",
    "forced-colors-mode",
    # No-layout/no-rendering HTML suites. These are useful for browser-renderer
    # fidelity investigations, but they dominate the broad WPT-cross signal
    # with layout, viewport visibility, media timeline, or render-blocking
    # expectations that Moli does not currently model. Do not put CSS
    # parser/CSSOM/computed-style suites here: Moli has a style engine
    # and those remain part of the default semantic baseline.
    "html/browsers/browsing-the-web/read-media",
    "html/browsers/browsing-the-web/scroll-to-fragid",
    "html/dom/render-blocking",
    "html/semantics/embedded-content/bfcache",
    "html/semantics/embedded-content/media-elements",
    "html/semantics/embedded-content/the-audio-element",
    "html/semantics/embedded-content/the-canvas-element",
    "html/semantics/embedded-content/the-video-element",
    "html/webappapis/update-rendering",
    # Internal/support WPT directories are useful for harness development, not
    # product capability baselines.
    "resources",
    "tools",
    # Browser scheduling features tied to rendering/prerendering.
    "long-animation-frame",
    "speculation-rules",
    # Device/platform integrations are not modeled by the CLI WPT runner.
    "accelerometer",
    "ambient-light",
    "compute-pressure",
    "gamepad",
    "notifications",
    "speech-api",
    "virtual-keyboard",
)

DEFAULT_EXCLUDE_CASE_PREFIXES: tuple[str, ...] = (
    # Active animation/timeline/interpolation behavior is not part of the
    # default semantic baseline. Keep parser/CSSOM subdirectories in the
    # default run through _is_default_non_goal_case exceptions below.
    "css/css-properties-values-api/animation/",
    "css/css-values/animations/",
    "css/css-values/calc-size/animation/",
    "css/css-values/attr-security-transition.html",
    "css/fill-stroke/animation/",
    "css/css-size-adjust/animations/",
    "css/css-color-hdr/interpolation.html",
    # Layout/geometry-heavy CSSOM surfaces.
    "css/css-conditional/at-supports-named-feature-001.html",
    "css/cssom/caretPositionFromPoint",
    "css/cssom/getComputedStyle-insets-fixed.html",
    "css/cssom/getComputedStyle-sticky-pos-percent.html",
    # DOM/editing cases whose style-related failures are transition,
    # selection/editing, or cross-document geometry behavior.
    "dom/nodes/moveBefore/continue-css-transition",
    "dom/nodes/moveBefore/css-transition",
    "editing/other/inserthtml-do-not-preserve-inline-styles.html",
    "selection/script-and-style-elements.html",
    # Malformed select fragment tag materialization would require owning or
    # replacing html5ever fragment tree construction internals. It is
    # documented as a parser non-goal in the WPT priority notes.
    "html/syntax/parsing/html5lib_innerHTML_webkit02.html",
    # Exact preservation of unpaired UTF-16 surrogates in reflected form
    # control name/value state would require non-scalar DOM backing. The
    # scalar replacement behavior is an explicit product non-goal.
    "xhr/formdata/constructor-formelement.html",
    # Click-derived image submitter coordinates depend on layout geometry,
    # which is outside the default headless semantic baseline.
    "xhr/formdata/constructor-submitter-coordinate.html",
    # SVG visual geometry with CSS styles remains available through focused
    # runs, but should not affect the broad headless baseline.
    "svg/geometry/svg-image-intrinsic-size-with-cssstyle-auto",
)

HTML_EXCLUDE_SUBSTRINGS: tuple[str, ...] = (
    ".tentative.",
    ".optional.",
    ".https.",
    ".h2.",
    ".sub.",
    ".serviceworker.",
    ".window.",
    ".any.",
    ".worker.",
)

EXCLUDE_SUBSTRINGS = HTML_EXCLUDE_SUBSTRINGS

ANY_JS_EXCLUDE_SUBSTRINGS: tuple[str, ...] = (
    ".tentative.",
    ".optional.",
    ".h2.",
    ".sub.",
    ".serviceworker.",
    ".window.",
    ".worker.",
)

WINDOW_JS_EXCLUDE_SUBSTRINGS: tuple[str, ...] = (
    ".tentative.",
    ".optional.",
    ".h2.",
    ".sub.",
    ".serviceworker.",
    ".any.",
    ".worker.",
)

DEFAULT_SERVICE_WORKER_FILENAME_ALLOW_TOKENS: tuple[str, ...] = (
    ".https.",
    ".sub.",
    ".serviceworker.",
)

ANY_JS_GLOBAL_CHOICES: tuple[str, ...] = (
    "none",
    ANY_JS_WINDOW_GLOBAL,
    ANY_JS_DEDICATED_WORKER_GLOBAL,
    "both",
)

# Script-authored WPT is excluded from the broad baseline by default. Opt in
# semantic, non-rendering suites here only when both wrapper globals are part
# of the runner contract. A non-``none`` ``--any-js-global`` remains an
# explicit override for focused investigations.
DEFAULT_ANY_JS_GLOBALS_BY_DIR_PREFIX: tuple[tuple[str, str], ...] = (
    ("streams", "both"),
)
DEFAULT_SCRIPT_CASE_DIR_PREFIXES: tuple[str, ...] = ("streams",)

# The initial layout baseline deliberately starts with the CSS areas that are
# both high-value for Moli and predominantly made up of deterministic, static
# documents. More areas can be opted into with --dir-prefix without changing
# the stable profile definition.
LAYOUT_PROFILE_DIR_PREFIXES: tuple[str, ...] = (
    "css/css-flexbox",
    "css/css-grid",
    "css/css-sizing",
    "css/cssom-view",
)

LAYOUT_DOCUMENT_SUFFIXES: tuple[str, ...] = (
    ".html",
    ".htm",
    ".xhtml",
    ".xht",
    ".svg",
    ".xml",
)

LAYOUT_DYNAMIC_PATH_PARTS = frozenset(
    {
        "animation",
        "animations",
        "media",
        "transitions",
    }
)
LAYOUT_MEDIA_ELEMENT_TAGS = frozenset({"audio", "canvas", "video"})


@dataclass(frozen=True)
class FuzzyTolerance:
    """WPT reftest fuzzy bounds.

    Each tuple is inclusive ``(minimum, maximum)`` as represented by WPT's
    ``maxDifference`` and ``totalPixels`` manifest metadata.
    """

    max_difference: tuple[int, int]
    total_pixels: tuple[int, int]

    def to_dict(self) -> dict[str, list[int]]:
        return {
            "max_difference": list(self.max_difference),
            "total_pixels": list(self.total_pixels),
        }


@dataclass(frozen=True)
class ReftestReference:
    reference_path: str
    relation: str
    fuzzy: FuzzyTolerance | None = None


@dataclass
class WptCase:
    case_path: str  # relative to wpt root, e.g. "dom/nodes/Node-textContent.html"
    timeout_multiplier: float = 1.0
    test_type: str = "testharness"
    references: tuple[ReftestReference, ...] = ()


@dataclass
class AnyJsMeta:
    variants: list[str]
    scripts: list[str]
    titles: list[str]
    timeout_multiplier: float
    globals: set[str]


UNSUPPORTED_SERVER_FEATURE_SUBSTRINGS: tuple[str, ...] = (".py",)
SUPPORTED_WASM_WEBAPI_WPTSERVE_HANDLER_REFERENCES: tuple[str, ...] = (
    "/fetch/api/resources/redirect.py",
    "/wasm/webapi/status.py",
    "/wasm/webapi/webapi/status.py",
    "wasm/webapi/status.py",
    "wasm/webapi/webapi/status.py",
    "webapi/status.py",
    "status.py",
)
WPTSERVE_HANDLER_TRAILING_BOUNDARY = r"(?=$|[?#'\"`)\]\}}\s,;])"
SUPPORTED_WASM_WEBAPI_WPTSERVE_HANDLER_PATTERNS: tuple[re.Pattern[str], ...] = tuple(
    re.compile(
        rf"(?<![A-Za-z0-9_./-]){re.escape(reference)}"
        rf"{WPTSERVE_HANDLER_TRAILING_BOUNDARY}"
    )
    for reference in SUPPORTED_WASM_WEBAPI_WPTSERVE_HANDLER_REFERENCES
)
SUPPORTED_XHR_DELAY_WPTSERVE_HANDLER_REFERENCES: tuple[str, ...] = (
    "/xhr/resources/delay.py",
    "xhr/resources/delay.py",
    "resources/delay.py",
    "delay.py",
)
SUPPORTED_XHR_DELAY_WPTSERVE_HANDLER_PATTERNS: tuple[re.Pattern[str], ...] = tuple(
    re.compile(
        rf"(?<![A-Za-z0-9_./-]){re.escape(reference)}"
        rf"{WPTSERVE_HANDLER_TRAILING_BOUNDARY}"
    )
    for reference in SUPPORTED_XHR_DELAY_WPTSERVE_HANDLER_REFERENCES
)
SUPPORTED_MODULE_DELAY_WPTSERVE_HANDLER_REFERENCES: tuple[str, ...] = (
    (
        "/html/semantics/scripting-1/the-script-element/module/"
        "resources/delayed-modulescript.py"
    ),
    "./resources/delayed-modulescript.py",
    "resources/delayed-modulescript.py",
    "./delayed-modulescript.py",
    "delayed-modulescript.py",
)
SUPPORTED_MODULE_DELAY_WPTSERVE_HANDLER_PATTERNS: tuple[re.Pattern[str], ...] = tuple(
    re.compile(
        rf"(?<![A-Za-z0-9_./-]){re.escape(reference)}"
        rf"{WPTSERVE_HANDLER_TRAILING_BOUNDARY}"
    )
    for reference in SUPPORTED_MODULE_DELAY_WPTSERVE_HANDLER_REFERENCES
)
CORE_HARNESS_SCRIPT_PATHS: tuple[str, ...] = (
    "/resources/testharness.js",
    "/resources/testharnessreport.js",
)
PRELOAD_HELPER_WPTSERVE_REFERENCE_ALLOWED_CASES: tuple[str, ...] = (
    "preload/avoid-delaying-onload-link-modulepreload-exec.html",
    "preload/avoid-delaying-onload-link-modulepreload.html",
)
PRELOAD_HELPER_SCRIPT_PATH = "preload/resources/preload_helper.js"
EXPLICIT_DIR_PREFIX_NON_GOAL_CASES: tuple[str, ...] = (
    # SRI correctness is outside the current Moli product boundary; keep
    # integrity metadata plumbing in code, but do not count hash mismatch WPTs
    # as focused preload goals.
    "preload/modulepreload-sri-importmap.html",
    "preload/modulepreload-sri.html",
    # Keep the malformed select fragment parser non-goal out of focused
    # parser slices too; see the WPT priority notes for the rationale.
    "html/syntax/parsing/html5lib_innerHTML_webkit02.html",
    # This file bundles otherwise-supported FormData behavior with exact lone
    # surrogate preservation, so keep it out of focused xhr slices as well.
    "xhr/formdata/constructor-formelement.html",
    # Keep the layout-derived image coordinate case out of focused xhr slices;
    # zero-coordinate FormData construction remains covered separately.
    "xhr/formdata/constructor-submitter-coordinate.html",
)

RAW_START_TAG_ATTRIBUTE_RE = re.compile(
    r"""\s+([^\s/>=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]*)))?""",
    re.DOTALL,
)
TERMINATED_HTML_CHARACTER_REFERENCE_RE = re.compile(
    r"&(?:#[xX][0-9A-Fa-f]+|#[0-9]+|[A-Za-z][A-Za-z0-9]+);"
)


def _raw_start_tag_attributes(start_tag_text: str) -> dict[str, str]:
    attributes: dict[str, str] = {}
    tag_name = re.match(r"<\s*[^\s/>]+", start_tag_text)
    if tag_name is None:
        return attributes
    for match in RAW_START_TAG_ATTRIBUTE_RE.finditer(start_tag_text, tag_name.end()):
        value = next(
            (candidate for candidate in match.groups()[1:] if candidate is not None),
            "",
        )
        attributes[match.group(1).lower()] = value
    return attributes


def _decode_raw_html_attribute_value(value: str) -> str:
    return TERMINATED_HTML_CHARACTER_REFERENCE_RE.sub(
        lambda match: unescape(match.group(0)),
        value,
    )


class _CaseHtmlParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.variants: list[str] = []
        self.script_srcs: list[str] = []
        self.element_tags: set[str] = set()
        self.long_timeout = False

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        tag_name = tag.lower()
        self.element_tags.add(tag_name)
        values = {name.lower(): value or "" for name, value in attrs}
        name = values.get("name", "").lower()
        if tag_name == "meta" and name == "variant":
            raw_values = _raw_start_tag_attributes(self.get_starttag_text() or "")
            raw_content = raw_values.get("content")
            self.variants.append(
                values.get("content", "")
                if raw_content is None
                else _decode_raw_html_attribute_value(raw_content)
            )
        elif (
            tag_name == "meta"
            and name == "timeout"
            and values.get("content", "").lower() == "long"
        ):
            self.long_timeout = True
        elif tag_name == "script":
            src = values.get("src", "")
            if src:
                self.script_srcs.append(src)


def _parse_case_html(html: str) -> _CaseHtmlParser:
    parser = _CaseHtmlParser()
    parser.feed(html)
    return parser


LAYOUT_ANIMATION_SOURCE_RE = re.compile(
    r"(?:"
    r"@(?:-\w+-)?keyframes\b"
    r"|(?:^|[;{])\s*(?:-\w+-)?(?:animation|transition)(?:-[\w-]+)?\s*:"
    r"|\.animate\s*\("
    r"|new\s+Animation\s*\("
    r")",
    re.IGNORECASE | re.MULTILINE,
)


def _has_layout_dynamic_dependency(
    rel_or_url: str,
    source: str,
    parser: _CaseHtmlParser,
) -> bool:
    """Return whether a layout case needs an intentionally excluded stack."""

    path = urlsplit(rel_or_url).path
    parts = {part.lower() for part in path.split("/") if part}
    if parts & LAYOUT_DYNAMIC_PATH_PARTS:
        return True
    if ".h2." in path.lower():
        return True
    if parser.element_tags & LAYOUT_MEDIA_ELEMENT_TAGS:
        return True
    return LAYOUT_ANIMATION_SOURCE_RE.search(HTML_COMMENT_RE.sub("", source)) is not None


def _script_timeout_multiplier(source: str) -> float:
    for line in source.splitlines():
        line = line.lstrip()
        if not line.startswith("//"):
            break
        meta = line.removeprefix("//").lstrip()
        if not meta.startswith("META:"):
            break
        if meta.removeprefix("META:").strip() == "timeout=long":
            return LONG_TIMEOUT_MULTIPLIER
    return 1.0


def _case_paths_for_variants(rel: str, parser: _CaseHtmlParser) -> list[str]:
    if not parser.variants:
        return [rel]
    paths = []
    for variant in parser.variants:
        if variant.startswith(("?", "#")):
            paths.append(f"{rel}{variant}")
        elif variant:
            paths.append(f"{rel}?{variant}")
        else:
            paths.append(rel)
    return paths


HTML_COMMENT_RE = re.compile(r"<!--.*?-->", re.DOTALL)
ANY_JS_META_RE = re.compile(r"^//\s*META:\s*(\w*)=(.*)$")


def parse_any_js_meta(source: str) -> AnyJsMeta:
    variants: list[str] = []
    scripts: list[str] = []
    titles: list[str] = []
    globals: set[str] = set()
    long_timeout = False
    for line in source.splitlines():
        match = ANY_JS_META_RE.match(line)
        if match is None:
            break
        name = match.group(1).strip().lower()
        value = match.group(2)
        if name == "variant":
            variants.append(value)
        elif name == "script":
            scripts.append(value)
        elif name == "title":
            titles.append(value)
        elif name == "timeout" and value.lower() == "long":
            long_timeout = True
        elif name == "global":
            globals.update(
                part.strip().lower()
                for part in value.split(",")
                if part.strip()
            )
    return AnyJsMeta(
        variants=variants,
        scripts=scripts,
        titles=titles,
        timeout_multiplier=LONG_TIMEOUT_MULTIPLIER if long_timeout else 1.0,
        globals=globals,
    )


def any_js_window_case_path(case_path: str) -> str:
    """Return the WPT-style generated window wrapper path for ``.any.js``."""

    before_fragment, sep, fragment = case_path.partition("#")
    before_query, query_sep, query = before_fragment.partition("?")
    if before_query.endswith(".any.js"):
        before_query = before_query.removesuffix(".any.js") + ".any.html"
    has_window_query = query_sep and any(
        part.split("=", 1)[0] == ANY_JS_WINDOW_QUERY_NAME
        for part in query.split("&")
    )
    if not has_window_query:
        if query_sep:
            query = f"{query}&{ANY_JS_WINDOW_QUERY}" if query else ANY_JS_WINDOW_QUERY
        else:
            query_sep = "?"
            query = ANY_JS_WINDOW_QUERY
    if query_sep:
        wrapped = f"{before_query}?{query}"
    else:
        wrapped = f"{before_query}?{ANY_JS_WINDOW_QUERY}"
    if sep:
        wrapped = f"{wrapped}#{fragment}"
    return wrapped


def window_js_window_case_path(case_path: str) -> str:
    """Return the WPT-style generated window wrapper path for ``.window.js``."""

    before_fragment, sep, fragment = case_path.partition("#")
    before_query, query_sep, query = before_fragment.partition("?")
    if before_query.endswith(".window.js"):
        before_query = before_query.removesuffix(".window.js") + ".window.html"
    has_window_query = query_sep and any(
        part.split("=", 1)[0] == WINDOW_JS_WINDOW_QUERY_NAME
        for part in query.split("&")
    )
    if not has_window_query:
        if query_sep:
            query = f"{query}&{WINDOW_JS_WINDOW_QUERY}" if query else WINDOW_JS_WINDOW_QUERY
        else:
            query_sep = "?"
            query = WINDOW_JS_WINDOW_QUERY
    if query_sep:
        wrapped = f"{before_query}?{query}"
    else:
        wrapped = f"{before_query}?{WINDOW_JS_WINDOW_QUERY}"
    if sep:
        wrapped = f"{wrapped}#{fragment}"
    return wrapped


def _filtered_exclude_tokens(
    tokens: tuple[str, ...],
    *,
    include_tentative: bool,
) -> tuple[str, ...]:
    if include_tentative:
        return tuple(token for token in tokens if token != ".tentative.")
    return tokens


def _html_path_is_supported(
    rel: str,
    *,
    explicit_dir_prefix: bool,
    include_tentative: bool,
) -> bool:
    excluded = HTML_EXCLUDE_SUBSTRINGS
    if explicit_dir_prefix:
        excluded = tuple(
            token
            for token in excluded
            if token
            not in {
                ".https.",
                ".sub.",
                ".serviceworker.",
            }
        )
    elif _is_default_service_worker_case(rel):
        excluded = tuple(
            token
            for token in excluded
            if token not in DEFAULT_SERVICE_WORKER_FILENAME_ALLOW_TOKENS
        )
    excluded = _filtered_exclude_tokens(
        excluded,
        include_tentative=include_tentative,
    )
    return not any(token in rel for token in excluded)


def _supported_wptserve_handler_references(
    rel: str | None,
) -> tuple[re.Pattern[str], ...]:
    supported: tuple[re.Pattern[str], ...] = ()
    if rel is not None and rel.startswith("wasm/webapi/"):
        supported += SUPPORTED_WASM_WEBAPI_WPTSERVE_HANDLER_PATTERNS
    if rel is not None and rel.startswith("xhr/"):
        supported += SUPPORTED_XHR_DELAY_WPTSERVE_HANDLER_PATTERNS
    if rel is not None and rel.startswith(
        "html/semantics/scripting-1/the-script-element/module/"
    ):
        supported += SUPPORTED_MODULE_DELAY_WPTSERVE_HANDLER_PATTERNS
    return supported


def _has_unsupported_server_feature(
    text: str,
    *,
    strip_html_comments: bool = False,
    rel: str | None = None,
) -> bool:
    if strip_html_comments:
        text = HTML_COMMENT_RE.sub("", text)
    for supported in _supported_wptserve_handler_references(rel):
        text = supported.sub("", text)
    return any(token in text for token in UNSUPPORTED_SERVER_FEATURE_SUBSTRINGS)


def _local_wpt_resource_path(wpt_root: Path, case_path: Path, src: str) -> Path | None:
    src_without_fragment = src.split("#", 1)[0].split("?", 1)[0]
    if not src_without_fragment or ":" in src_without_fragment:
        return None
    if src_without_fragment.startswith("//"):
        return None
    if src_without_fragment.startswith("/"):
        return wpt_root / src_without_fragment.lstrip("/")
    return case_path.parent / src_without_fragment


def _references_unsupported_server_feature(
    wpt_root: Path,
    case_path: Path,
    html: str,
    parser: _CaseHtmlParser,
) -> bool:
    rel = case_path.relative_to(wpt_root).as_posix()
    if _has_unsupported_server_feature(html, strip_html_comments=True, rel=rel):
        return True
    for src in parser.script_srcs:
        if src.split("#", 1)[0].split("?", 1)[0] in CORE_HARNESS_SCRIPT_PATHS:
            continue
        resource_path = _local_wpt_resource_path(wpt_root, case_path, src)
        if resource_path is None or not resource_path.is_file():
            continue
        try:
            resource_path.resolve().relative_to(wpt_root)
        except ValueError:
            continue
        try:
            script = resource_path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        script_rel = resource_path.relative_to(wpt_root).as_posix()
        if (
            script_rel == PRELOAD_HELPER_SCRIPT_PATH
            and rel in PRELOAD_HELPER_WPTSERVE_REFERENCE_ALLOWED_CASES
        ):
            continue
        if _has_unsupported_server_feature(script, rel=script_rel):
            return True
    return False


def _any_js_references_unsupported_server_feature(
    wpt_root: Path,
    case_path: Path,
    source: str,
    meta: AnyJsMeta,
) -> bool:
    rel = case_path.relative_to(wpt_root).as_posix()
    if _has_unsupported_server_feature(source, rel=rel):
        return True
    for src in meta.scripts:
        if src.split("#", 1)[0].split("?", 1)[0] in CORE_HARNESS_SCRIPT_PATHS:
            continue
        resource_path = _local_wpt_resource_path(wpt_root, case_path, src)
        if resource_path is None or not resource_path.is_file():
            continue
        try:
            resource_path.resolve().relative_to(wpt_root)
        except ValueError:
            return True
        try:
            script = resource_path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        script_rel = resource_path.relative_to(wpt_root).as_posix()
        if _has_unsupported_server_feature(script, rel=script_rel):
            return True
    return False


def _any_js_allows_window(meta: AnyJsMeta) -> bool:
    return not meta.globals or "window" in meta.globals


def _any_js_allows_worker(meta: AnyJsMeta) -> bool:
    return not meta.globals or bool(meta.globals & {"worker", "dedicatedworker"})


def enumerate_cases(
    wpt_root: Path,
    *,
    dir_prefixes: tuple[str, ...] | None = None,
    exclude_dir_prefixes: tuple[str, ...] = DEFAULT_EXCLUDE_DIR_PREFIXES,
    exclude_case_prefixes: tuple[str, ...] = DEFAULT_EXCLUDE_CASE_PREFIXES,
    include_tentative: bool = False,
    any_js_global: str = "none",
    layout_static_only: bool = False,
    limit: int | None = None,
) -> list[WptCase]:
    """Walk ``wpt_root`` and return selectable cases.

    Cases are returned in deterministic sorted order so that re-runs and
    cross-engine runs see the exact same list.
    """

    wpt_root = wpt_root.resolve()
    if not wpt_root.exists():
        raise RuntimeError(f"wpt root does not exist: {wpt_root}")
    if any_js_global not in ANY_JS_GLOBAL_CHOICES:
        raise ValueError(f"unsupported .any.js global selection: {any_js_global}")

    found: list[WptCase] = []
    prefixes = dir_prefixes if dir_prefixes is not None else ("",)
    for prefix in prefixes:
        base = wpt_root / prefix
        if not base.exists():
            continue
        for path in base.rglob("*.html"):
            if not path.is_file():
                continue
            rel = path.relative_to(wpt_root).as_posix()
            if dir_prefixes is None and _matches_any_dir_prefix(rel, exclude_dir_prefixes):
                continue
            if dir_prefixes is None and _is_default_non_goal_case(
                rel,
                exclude_case_prefixes=exclude_case_prefixes,
            ):
                continue
            if dir_prefixes is not None and rel in EXPLICIT_DIR_PREFIX_NON_GOAL_CASES:
                continue
            if _is_manual_or_support_case(rel):
                continue
            sibling_py = path.with_suffix(".py")
            if sibling_py.exists():
                continue
            try:
                head = path.read_text(encoding="utf-8", errors="ignore")
            except OSError:
                continue
            if not _html_path_is_supported(
                rel,
                explicit_dir_prefix=dir_prefixes is not None,
                include_tentative=include_tentative,
            ):
                continue
            if "/resources/testharness.js" not in head:
                continue
            if "/resources/testdriver" in head:
                continue
            parser = _parse_case_html(head)
            if _references_unsupported_server_feature(wpt_root, path, head, parser):
                continue
            if layout_static_only and _has_layout_dynamic_dependency(rel, head, parser):
                continue
            timeout_multiplier = LONG_TIMEOUT_MULTIPLIER if parser.long_timeout else 1.0
            found.extend(
                WptCase(case_path=case_path, timeout_multiplier=timeout_multiplier)
                for case_path in _case_paths_for_variants(rel, parser)
            )
        for path in base.rglob("*.any.js"):
            if not path.is_file():
                continue
            rel = path.relative_to(wpt_root).as_posix()
            selected_any_js_global = _selected_any_js_global(rel, any_js_global)
            if selected_any_js_global == "none":
                continue
            if not is_any_js_case_path(rel):
                continue
            if dir_prefixes is None and _matches_any_dir_prefix(rel, exclude_dir_prefixes):
                continue
            if dir_prefixes is None and _is_default_non_goal_case(
                rel,
                exclude_case_prefixes=exclude_case_prefixes,
            ):
                continue
            if _is_manual_or_support_case(rel):
                continue
            if _is_excluded_by_substring(
                rel,
                include_tentative=include_tentative,
                include_any_js=True,
                include_sub_https=dir_prefixes is not None,
            ):
                continue
            try:
                source = path.read_text(encoding="utf-8", errors="ignore")
            except OSError:
                continue
            if "/resources/testdriver" in source:
                continue
            meta = parse_any_js_meta(source)
            if _any_js_references_unsupported_server_feature(wpt_root, path, source, meta):
                continue
            for variant_path in _case_paths_for_variants(rel, meta):
                for case_path in _any_js_case_paths_for_global(
                    variant_path,
                    selected_any_js_global,
                    meta,
                ):
                    found.append(
                        WptCase(
                            case_path=case_path,
                            timeout_multiplier=meta.timeout_multiplier,
                        )
                    )
        for pattern in ("*.window.js", "*.worker.js"):
            for path in base.rglob(pattern):
                if not path.is_file():
                    continue
                rel = path.relative_to(wpt_root).as_posix()
                if dir_prefixes is None and not _matches_any_dir_prefix(
                    rel,
                    DEFAULT_SCRIPT_CASE_DIR_PREFIXES,
                ):
                    continue
                if not is_script_js_case_path(rel):
                    continue
                if _is_manual_or_support_case(rel):
                    continue
                if _is_excluded_by_substring(
                    rel,
                    include_tentative=include_tentative,
                    include_script_js=True,
                    include_sub_https=True,
                ):
                    continue
                try:
                    source = path.read_text(encoding="utf-8", errors="ignore")
                except OSError:
                    continue
                if "/resources/testdriver" in source:
                    continue
                meta = parse_any_js_meta(source)
                if _any_js_references_unsupported_server_feature(
                    wpt_root,
                    path,
                    source,
                    meta,
                ):
                    continue
                for variant_path in _case_paths_for_variants(rel, meta):
                    found.append(
                        WptCase(
                            case_path=normalize_script_js_case_path(variant_path),
                            timeout_multiplier=meta.timeout_multiplier,
                        )
                    )

    found.sort(key=lambda case: case.case_path)
    if limit is not None:
        found = found[:limit]
    return found


def _is_excluded_by_substring(
    rel: str,
    *,
    include_tentative: bool,
    include_any_js: bool = False,
    include_script_js: bool = False,
    include_sub_https: bool = False,
) -> bool:
    for token in EXCLUDE_SUBSTRINGS:
        if token == ".tentative." and include_tentative:
            continue
        if token == ".any." and include_any_js:
            continue
        if token in {".window.", ".worker."} and include_script_js:
            continue
        if token in {".sub.", ".https.", ".serviceworker."} and include_sub_https:
            continue
        if token in rel:
            return True
    return False


def _is_default_service_worker_case(rel: str) -> bool:
    rel_lower = rel.lower()
    return "service-worker" in rel_lower or "serviceworker" in rel_lower


def _any_js_case_paths_for_global(
    rel: str,
    any_js_global: str,
    meta: AnyJsMeta,
) -> list[str]:
    if any_js_global == ANY_JS_WINDOW_GLOBAL and _any_js_allows_window(meta):
        return [normalize_any_js_window_case_path(rel)]
    if any_js_global == ANY_JS_DEDICATED_WORKER_GLOBAL and _any_js_allows_worker(meta):
        return [any_js_case_path_for_global(rel, ANY_JS_DEDICATED_WORKER_GLOBAL)]
    if any_js_global == "both":
        paths = []
        if _any_js_allows_worker(meta):
            paths.append(any_js_case_path_for_global(rel, ANY_JS_DEDICATED_WORKER_GLOBAL))
        if _any_js_allows_window(meta):
            paths.append(normalize_any_js_window_case_path(rel))
        return paths
    return []


def _selected_any_js_global(rel: str, requested_global: str) -> str:
    if requested_global != "none":
        return requested_global
    for prefix, default_global in DEFAULT_ANY_JS_GLOBALS_BY_DIR_PREFIX:
        if _matches_any_dir_prefix(rel, (prefix,)):
            return default_global
    return "none"


def _matches_any_dir_prefix(rel: str, prefixes: tuple[str, ...]) -> bool:
    return any(
        not prefix or rel == prefix or rel.startswith(f"{prefix}/")
        for prefix in prefixes
    )


def _matches_any_case_prefix(rel: str, prefixes: tuple[str, ...]) -> bool:
    return any(rel.startswith(prefix) for prefix in prefixes)


def _is_default_non_goal_case(
    rel: str,
    *,
    exclude_case_prefixes: tuple[str, ...],
) -> bool:
    if _matches_any_case_prefix(rel, exclude_case_prefixes):
        return True
    if rel.startswith("css/css-transitions/"):
        return not rel.startswith("css/css-transitions/parsing/")
    if rel.startswith("css/css-animations/"):
        return not rel.startswith(
            (
                "css/css-animations/parsing/",
                "css/css-animations/stability/",
            )
        )
    if rel.startswith("css/css-conditional/container-queries/"):
        return not _is_default_goal_container_query_case(rel)
    if rel.startswith("css/css-viewport/"):
        return not rel.startswith("css/css-viewport/zoom/parsing/")
    if rel.startswith("css/css-scrollbars/"):
        return not (rel.endswith("/inheritance.html") or "parsing" in rel)
    if rel.startswith("css/css-color-adjust/"):
        return not (rel.endswith("/inheritance.html") or "/parsing/" in rel)
    if rel.startswith("css/compositing/"):
        return not (rel.endswith("/inheritance.html") or "/parsing/" in rel)
    if rel.startswith("css/css-content/"):
        return "animation" in rel or "interpolation" in rel
    if rel.startswith("css/css-forms/"):
        return "/parsing/" not in rel
    if rel.startswith("css/css-overscroll-behavior/"):
        return "/parsing/" not in rel
    if rel.startswith("css/css-forced-color-adjust/"):
        return "/parsing/" not in rel
    return False


def _is_default_goal_container_query_case(rel: str) -> bool:
    name = rel.removeprefix("css/css-conditional/container-queries/")
    if name.startswith("scroll-state/"):
        scroll_name = name.removeprefix("scroll-state/")
        return scroll_name.startswith("at-container-") or scroll_name in (
            "container-type-scroll-state-computed.html",
            "container-type-scroll-state-parsing.html",
        )
    goal_tokens = (
        "parsing",
        "serialization",
        "computed",
        "idlharness",
        "container-rule-cssom",
        "container-inner-at-rules",
        "container-ident-function",
        "container-inheritance",
        "container-longhand-animation-type",
        "container-name",
        "container-parsing",
        "container-type-parsing",
        "custom-property-style",
        "display-contents-dynamic-style",
        "multiple-style-containers",
        "nested-style-size",
        "query-evaluation-style",
        "registered-color-style",
        "style-container",
        "style-query",
        "style-queries",
    )
    return any(token in name for token in goal_tokens)


def _is_manual_or_support_case(rel: str) -> bool:
    parts = rel.split("/")
    return parts[-1].endswith("-manual.html") or any(
        part in {"resources", "support"} for part in parts[:-1]
    )


def _iter_manifest_leaves(
    node: Any,
    parts: tuple[str, ...] = (),
) -> Iterator[tuple[str, list[Any]]]:
    if isinstance(node, list):
        yield "/".join(parts), node
        return
    if not isinstance(node, dict):
        return
    for name in sorted(node):
        yield from _iter_manifest_leaves(node[name], (*parts, name))


def _load_reftest_manifest(wpt_root: Path) -> dict[str, Any]:
    manifest_path = wpt_root / "MANIFEST.json"
    if not manifest_path.is_file():
        raise RuntimeError(f"WPT manifest does not exist: {manifest_path}")
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise RuntimeError(f"failed to read WPT manifest {manifest_path}: {exc}") from exc
    reftests = (manifest.get("items") or {}).get("reftest")
    if not isinstance(reftests, dict):
        raise RuntimeError(f"WPT manifest has no reftest items: {manifest_path}")
    return reftests


def _manifest_test_url(source_rel: str, raw_url: Any) -> str | None:
    if raw_url is None:
        value = source_rel
    elif not isinstance(raw_url, str) or not raw_url:
        return None
    elif raw_url.startswith(("?", "#")):
        value = f"{source_rel}{raw_url}"
    else:
        value = raw_url
    parsed = urlsplit(value)
    if parsed.scheme or parsed.netloc:
        return None
    path = parsed.path.lstrip("/")
    if not path or ".." in path.split("/"):
        return None
    suffix = f"?{parsed.query}" if parsed.query else ""
    if parsed.fragment:
        suffix += f"#{parsed.fragment}"
    return f"{path}{suffix}"


def _manifest_reference_url(test_url: str, raw_url: Any) -> str | None:
    if not isinstance(raw_url, str) or not raw_url:
        return None
    parsed_raw = urlsplit(raw_url)
    if parsed_raw.scheme or parsed_raw.netloc or raw_url.startswith("//"):
        return None
    joined = urlsplit(urljoin(f"/{test_url}", raw_url))
    path = joined.path.lstrip("/")
    if not path or ".." in path.split("/"):
        return None
    suffix = f"?{joined.query}" if joined.query else ""
    if joined.fragment:
        suffix += f"#{joined.fragment}"
    return f"{path}{suffix}"


def _fuzzy_range(value: Any) -> tuple[int, int] | None:
    if (
        not isinstance(value, list)
        or len(value) != 2
        or not all(isinstance(item, int) and not isinstance(item, bool) for item in value)
    ):
        return None
    minimum, upper = value
    if minimum < 0 or upper < minimum:
        return None
    return minimum, upper


def _manifest_fuzzy_tolerance(
    extras: dict[str, Any],
    *,
    test_url: str,
    reference_url: str,
    relation: str,
) -> FuzzyTolerance | None:
    fuzzy_entries = extras.get("fuzzy")
    if not isinstance(fuzzy_entries, list):
        return None

    default_ranges: Any = None
    keyed_ranges: Any = None
    canonical_key = (f"/{test_url}", f"/{reference_url}", relation)
    for entry in fuzzy_entries:
        if not isinstance(entry, list) or len(entry) != 2:
            continue
        key, ranges = entry
        if key is None:
            default_ranges = ranges
        elif isinstance(key, list) and tuple(key) == canonical_key:
            keyed_ranges = ranges
    ranges = keyed_ranges if keyed_ranges is not None else default_ranges
    if not isinstance(ranges, list) or len(ranges) != 2:
        return None
    max_difference = _fuzzy_range(ranges[0])
    total_pixels = _fuzzy_range(ranges[1])
    if max_difference is None or total_pixels is None:
        return None
    return FuzzyTolerance(
        max_difference=max_difference,
        total_pixels=total_pixels,
    )


def _layout_document_is_static(
    wpt_root: Path,
    document_url: str,
    cache: dict[str, tuple[bool, float]],
) -> tuple[bool, float]:
    document_path = urlsplit(document_url).path.lstrip("/")
    cached = cache.get(document_path)
    if cached is not None:
        return cached
    if (
        not document_path.lower().endswith(LAYOUT_DOCUMENT_SUFFIXES)
        or ".h2." in document_path.lower()
        or document_path.endswith(".py")
    ):
        cache[document_path] = (False, 1.0)
        return cache[document_path]
    file_path = (wpt_root / document_path).resolve()
    try:
        file_path.relative_to(wpt_root)
    except ValueError:
        cache[document_path] = (False, 1.0)
        return cache[document_path]
    if not file_path.is_file() or file_path.with_suffix(".py").exists():
        cache[document_path] = (False, 1.0)
        return cache[document_path]
    try:
        source = file_path.read_text(encoding="utf-8", errors="ignore")
    except OSError:
        cache[document_path] = (False, 1.0)
        return cache[document_path]
    parser = _parse_case_html(source)
    supported = not (
        "/resources/testdriver" in source
        or _references_unsupported_server_feature(wpt_root, file_path, source, parser)
        or _has_layout_dynamic_dependency(document_url, source, parser)
    )
    timeout_multiplier = LONG_TIMEOUT_MULTIPLIER if parser.long_timeout else 1.0
    cache[document_path] = (supported, timeout_multiplier)
    return cache[document_path]


def enumerate_reftest_cases(
    wpt_root: Path,
    *,
    dir_prefixes: tuple[str, ...] = LAYOUT_PROFILE_DIR_PREFIXES,
    include_tentative: bool = False,
    limit: int | None = None,
) -> list[WptCase]:
    """Enumerate deterministic static reftests from upstream ``MANIFEST.json``."""

    wpt_root = wpt_root.resolve()
    if not wpt_root.exists():
        raise RuntimeError(f"wpt root does not exist: {wpt_root}")
    reftest_manifest = _load_reftest_manifest(wpt_root)
    document_cache: dict[str, tuple[bool, float]] = {}
    found: list[WptCase] = []

    for source_rel, leaf in _iter_manifest_leaves(reftest_manifest):
        if not _matches_any_dir_prefix(source_rel, dir_prefixes):
            continue
        if not isinstance(leaf, list) or len(leaf) < 2:
            continue
        if not include_tentative and ".tentative." in source_rel:
            continue
        if ".optional." in source_rel or ".h2." in source_rel:
            continue

        for raw_item in leaf[1:]:
            if not isinstance(raw_item, list) or len(raw_item) != 3:
                continue
            raw_url, raw_references, raw_extras = raw_item
            if not isinstance(raw_references, list) or not isinstance(raw_extras, dict):
                continue
            if raw_extras.get("testdriver"):
                continue
            test_url = _manifest_test_url(source_rel, raw_url)
            if test_url is None:
                continue
            if not include_tentative and ".tentative." in test_url:
                continue
            test_supported, source_timeout_multiplier = _layout_document_is_static(
                wpt_root,
                test_url,
                document_cache,
            )
            if not test_supported:
                continue

            references: list[ReftestReference] = []
            references_supported = True
            for raw_reference in raw_references:
                if not isinstance(raw_reference, list) or len(raw_reference) != 2:
                    references_supported = False
                    break
                raw_reference_url, relation = raw_reference
                if relation not in {"==", "!="}:
                    references_supported = False
                    break
                reference_url = _manifest_reference_url(test_url, raw_reference_url)
                if reference_url is None:
                    references_supported = False
                    break
                reference_supported, _ = _layout_document_is_static(
                    wpt_root,
                    reference_url,
                    document_cache,
                )
                if not reference_supported:
                    references_supported = False
                    break
                references.append(
                    ReftestReference(
                        reference_path=reference_url,
                        relation=relation,
                        fuzzy=_manifest_fuzzy_tolerance(
                            raw_extras,
                            test_url=test_url,
                            reference_url=reference_url,
                            relation=relation,
                        ),
                    )
                )
            if not references_supported or not references:
                continue
            timeout_multiplier = (
                LONG_TIMEOUT_MULTIPLIER
                if raw_extras.get("timeout") == "long"
                else source_timeout_multiplier
            )
            found.append(
                WptCase(
                    case_path=test_url,
                    timeout_multiplier=timeout_multiplier,
                    test_type="reftest",
                    references=tuple(references),
                )
            )

    found.sort(key=lambda case: case.case_path)
    if limit is not None:
        found = found[:limit]
    return found


def explicit_reftest_case(wpt_root: Path, case_path: str) -> WptCase:
    normalized = case_path.lstrip("/")
    file_path = urlsplit(normalized).path
    parent = str(Path(file_path).parent).replace("\\", "/")
    if parent == ".":
        parent = ""
    for case in enumerate_reftest_cases(
        wpt_root,
        dir_prefixes=(parent,),
        include_tentative=True,
    ):
        if case.case_path == normalized:
            return case
    raise RuntimeError(f"WPT reftest is not selectable: {case_path}")


def explicit_case(wpt_root: Path, case_path: str) -> WptCase:
    case_path = normalize_any_js_window_case_path(case_path)
    case_path = normalize_script_js_case_path(case_path)
    file_case_path = urlsplit(case_path).path.lstrip("/")
    if not file_case_path or ".." in file_case_path.split("/"):
        raise RuntimeError(f"invalid WPT case path: {case_path}")
    source_case_path = file_case_path
    is_any_js_case = file_case_path.endswith(".any.js")
    is_script_js_case = is_script_js_case_path(case_path)
    if file_case_path.endswith(".any.html"):
        source_case_path = file_case_path.removesuffix(".any.html") + ".any.js"
        is_any_js_case = True
    elif file_case_path.endswith(".window.html"):
        source_case_path = file_case_path.removesuffix(".window.html") + ".window.js"
        is_script_js_case = True
    file_path = (wpt_root / source_case_path).resolve()
    try:
        file_path.relative_to(wpt_root.resolve())
    except ValueError as exc:
        raise RuntimeError(f"invalid WPT case path: {case_path}") from exc
    if not file_path.exists() or not file_path.is_file():
        raise RuntimeError(f"WPT case does not exist: {case_path}")
    source = file_path.read_text(encoding="utf-8", errors="ignore")[:200_000]
    normalized_case_path = case_path.lstrip("/")
    if is_any_js_case or is_script_js_case:
        timeout_multiplier = _script_timeout_multiplier(source)
    else:
        parser = _parse_case_html(source)
        timeout_multiplier = LONG_TIMEOUT_MULTIPLIER if parser.long_timeout else 1.0
    return WptCase(case_path=normalized_case_path, timeout_multiplier=timeout_multiplier)
