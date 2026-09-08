"""Helpers for running WPT ``.any.js`` fixtures through a window wrapper."""

from __future__ import annotations

from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

ANY_JS_WRAPPER_QUERY_KEY = "moli-wpt-any"
ANY_JS_WINDOW_GLOBAL = "window"
ANY_JS_DEDICATED_WORKER_GLOBAL = "dedicatedworker"
ANY_JS_WORKER_SCRIPT_SUFFIX = ".any.worker.js"
ANY_JS_SOURCE_SCRIPT_SUFFIX = ".any.js"
SCRIPT_JS_WRAPPER_QUERY_KEY = "moli-wpt-script"
SCRIPT_JS_WINDOW_GLOBAL = "window"
SCRIPT_JS_DEDICATED_WORKER_GLOBAL = "dedicatedworker"
WINDOW_JS_SOURCE_SCRIPT_SUFFIX = ".window.js"
WORKER_JS_SOURCE_SCRIPT_SUFFIX = ".worker.js"


def is_any_js_case_path(case_path: str) -> bool:
    return urlsplit(case_path.lstrip("/")).path.endswith(ANY_JS_SOURCE_SCRIPT_SUFFIX)


def is_any_js_worker_script_path(case_path: str) -> bool:
    return urlsplit(case_path.lstrip("/")).path.endswith(ANY_JS_WORKER_SCRIPT_SUFFIX)


def any_js_wrapper_global(query: str) -> str | None:
    for key, value in parse_qsl(query, keep_blank_values=True):
        if key == ANY_JS_WRAPPER_QUERY_KEY:
            return value or ANY_JS_WINDOW_GLOBAL
    return None


def is_window_js_case_path(case_path: str) -> bool:
    return urlsplit(case_path.lstrip("/")).path.endswith(WINDOW_JS_SOURCE_SCRIPT_SUFFIX)


def is_worker_js_case_path(case_path: str) -> bool:
    return urlsplit(case_path.lstrip("/")).path.endswith(WORKER_JS_SOURCE_SCRIPT_SUFFIX)


def is_script_js_case_path(case_path: str) -> bool:
    return is_window_js_case_path(case_path) or is_worker_js_case_path(case_path)


def script_js_wrapper_global(query: str) -> str | None:
    for key, value in parse_qsl(query, keep_blank_values=True):
        if key == SCRIPT_JS_WRAPPER_QUERY_KEY:
            return value or SCRIPT_JS_WINDOW_GLOBAL
    return None


def normalize_any_js_window_case_path(case_path: str) -> str:
    """Return an explicit case path that asks the fixture server for a wrapper.

    WPT ``.any.js`` files are script resources, not directly navigable test
    pages. The benchmark opts selected semantic suites into script enumeration;
    an explicit ``--case path.any.js`` also runs the window-global variant so
    other suites, such as wasm/jsapi, remain measurable in focused runs.
    """

    case_path = case_path.lstrip("/")
    parts = urlsplit(case_path)
    if (
        not parts.path.endswith(".any.js")
        or any_js_wrapper_global(parts.query) is not None
    ):
        return case_path
    return any_js_case_path_for_global(case_path, ANY_JS_WINDOW_GLOBAL)


def normalize_script_js_case_path(case_path: str) -> str:
    case_path = case_path.lstrip("/")
    parts = urlsplit(case_path)
    if script_js_wrapper_global(parts.query) is not None:
        return case_path
    if parts.path.endswith(WINDOW_JS_SOURCE_SCRIPT_SUFFIX):
        return script_js_case_path_for_global(case_path, SCRIPT_JS_WINDOW_GLOBAL)
    if parts.path.endswith(WORKER_JS_SOURCE_SCRIPT_SUFFIX):
        return script_js_case_path_for_global(case_path, SCRIPT_JS_DEDICATED_WORKER_GLOBAL)
    return case_path


def any_js_case_path_for_global(case_path: str, global_name: str) -> str:
    """Return a wrapper case path for a specific ``.any.js`` global."""

    case_path = case_path.lstrip("/")
    parts = urlsplit(case_path)
    query_items = [
        (key, value)
        for key, value in parse_qsl(parts.query, keep_blank_values=True)
        if key != ANY_JS_WRAPPER_QUERY_KEY
    ]
    query_items.append((ANY_JS_WRAPPER_QUERY_KEY, global_name))
    return urlunsplit(("", "", parts.path, urlencode(query_items), parts.fragment))


def script_js_case_path_for_global(case_path: str, global_name: str) -> str:
    case_path = case_path.lstrip("/")
    parts = urlsplit(case_path)
    query_items = [
        (key, value)
        for key, value in parse_qsl(parts.query, keep_blank_values=True)
        if key != SCRIPT_JS_WRAPPER_QUERY_KEY
    ]
    query_items.append((SCRIPT_JS_WRAPPER_QUERY_KEY, global_name))
    return urlunsplit(("", "", parts.path, urlencode(query_items), parts.fragment))


def any_js_worker_script_path(case_path: str) -> str:
    """Return the WPT-style dedicated worker wrapper path for a ``.any.js`` case."""

    parts = urlsplit(case_path.lstrip("/"))
    if not parts.path.endswith(ANY_JS_SOURCE_SCRIPT_SUFFIX):
        return case_path.lstrip("/")
    worker_path = (
        parts.path.removesuffix(ANY_JS_SOURCE_SCRIPT_SUFFIX)
        + ANY_JS_WORKER_SCRIPT_SUFFIX
    )
    return urlunsplit(("", "", worker_path, parts.query, parts.fragment))


def any_js_source_script_path(worker_script_path: str) -> str:
    """Return the source ``.any.js`` path for a dedicated worker wrapper path."""

    parts = urlsplit(worker_script_path.lstrip("/"))
    if not parts.path.endswith(ANY_JS_WORKER_SCRIPT_SUFFIX):
        return worker_script_path.lstrip("/")
    source_path = (
        parts.path.removesuffix(ANY_JS_WORKER_SCRIPT_SUFFIX)
        + ANY_JS_SOURCE_SCRIPT_SUFFIX
    )
    return urlunsplit(("", "", source_path, parts.query, parts.fragment))


def query_without_any_js_wrapper(query: str) -> str:
    query_items = [
        (key, value)
        for key, value in parse_qsl(query, keep_blank_values=True)
        if key != ANY_JS_WRAPPER_QUERY_KEY
    ]
    return urlencode(query_items)


def query_without_script_js_wrapper(query: str) -> str:
    query_items = [
        (key, value)
        for key, value in parse_qsl(query, keep_blank_values=True)
        if key != SCRIPT_JS_WRAPPER_QUERY_KEY
    ]
    return urlencode(query_items)
