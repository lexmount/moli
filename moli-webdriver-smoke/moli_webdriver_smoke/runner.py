from __future__ import annotations

import argparse
import asyncio
import json
import os
import sys
import traceback
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Awaitable, Callable, Iterable

from .config import WebDriverTarget, clear_current_process_proxy_env, reserve_port
from .fixture import FixtureServer
from .groups.bidi import run_bidi_group
from .groups.classic import run_classic_group
from .groups.navigation_errors import run_navigation_errors_group
from .groups.script_interrupt import (
    run_chromedriver_script_timeout_group,
    run_moli_script_interrupt_group,
)
from .groups.selenium import run_selenium_group
from .groups.semantics import run_semantics_group
from .groups.url_policy import run_url_policy_group
from .scenarios import has_failures, record_failure
from .serve import (
    MoliServe,
    render_moli_serve_diagnostics,
    start_moli_serve,
    stop_moli_serve,
    wait_for_webdriver_server,
)


clear_current_process_proxy_env()


GroupRunner = Callable[
    [WebDriverTarget, str, list[dict[str, Any]], bool],
    Awaitable[None],
]


@dataclass(frozen=True)
class SmokeGroup:
    name: str
    description: str
    runner: GroupRunner
    default: bool = True


async def _run_classic_group(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    continue_on_failure: bool,
) -> None:
    await run_classic_group(
        target.endpoint,
        fixture,
        results,
        continue_on_failure,
    )


async def _run_bidi_group(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    continue_on_failure: bool,
) -> None:
    await run_bidi_group(
        target.endpoint,
        fixture,
        results,
        continue_on_failure,
    )


ALL_GROUPS: tuple[SmokeGroup, ...] = (
    SmokeGroup(
        "classic",
        "Raw WebDriver Classic HTTP session, navigation, element, script, alert, shadow, cookie, and window state flows.",
        _run_classic_group,
    ),
    SmokeGroup(
        "bidi",
        "Raw WebDriver BiDi WebSocket session, browsingContext lifecycle, input actions, element origins, file uploads, network data, user contexts, emulation, explicit screenshot unsupported errors, and storage.",
        _run_bidi_group,
        default=False,
    ),
    SmokeGroup(
        "selenium",
        "Selenium Python Remote WebDriver session, BiDi facade, navigation, elements, forms/files/actions, explicit screenshot/print unsupported errors, cookies, windows, frame switching, script arguments, shadow roots, and dialogs.",
        run_selenium_group,
        default=False,
    ),
    SmokeGroup(
        "semantics",
        "Isolated cross-engine WebDriver contracts for capabilities, history, storage, frames, dialogs, and W3C errors.",
        run_semantics_group,
        default=False,
    ),
    SmokeGroup(
        "url-policy",
        "Hosted file-navigation rejection with exact Classic/BiDi errors, unchanged contexts, and no BiDi lifecycle events.",
        run_url_policy_group,
        default=False,
    ),
    SmokeGroup(
        "navigation-errors",
        "Chromium/WPT Classic and BiDi navigation argument, missing-context, address-error, envelope, and post-failure liveness matrix.",
        run_navigation_errors_group,
        default=False,
    ),
    SmokeGroup(
        "script-timeout-chromium",
        "ChromeDriver Classic script-timeout yield boundary and repeated same-window recovery oracle.",
        run_chromedriver_script_timeout_group,
        default=False,
    ),
    SmokeGroup(
        "script-interrupt",
        "Moli Classic script timeout preemption of non-yielding sync/async JavaScript through renderer IO, with repeated same-window recovery.",
        run_moli_script_interrupt_group,
        default=False,
    ),
)
DEFAULT_GROUP_NAMES: tuple[str, ...] = tuple(group.name for group in ALL_GROUPS if group.default)
GROUPS_BY_NAME: dict[str, SmokeGroup] = {group.name: group for group in ALL_GROUPS}


def _split_group_names(raw_names: Iterable[str]) -> list[str]:
    names: list[str] = []
    for raw_name in raw_names:
        names.extend(name.strip() for name in raw_name.split(",") if name.strip())
    return names


def resolve_group_selection(raw_names: Iterable[str] = ()) -> tuple[SmokeGroup, ...]:
    names = _split_group_names(raw_names)
    if not names:
        env_names = os.environ.get("MOLI_WEBDRIVER_SMOKE_GROUPS", "")
        names = _split_group_names([env_names]) if env_names else list(DEFAULT_GROUP_NAMES)
    unknown = [name for name in names if name not in GROUPS_BY_NAME]
    if unknown:
        available = ", ".join(group.name for group in ALL_GROUPS)
        raise RuntimeError(f"unknown smoke group(s): {', '.join(unknown)}; available groups: {available}")
    selected: list[SmokeGroup] = []
    seen: set[str] = set()
    for name in names:
        if name in seen:
            continue
        selected.append(GROUPS_BY_NAME[name])
        seen.add(name)
    return tuple(selected)


def group_listing() -> list[dict[str, Any]]:
    return [
        {
            "name": group.name,
            "default": group.default,
            "description": group.description,
        }
        for group in ALL_GROUPS
    ]


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run WebDriver smoke workflows against moli.")
    parser.add_argument(
        "--group",
        action="append",
        default=[],
        help="Run only the named group. May be repeated or comma-separated. Defaults to the Classic group.",
    )
    parser.add_argument(
        "--list-groups",
        action="store_true",
        help="List available groups as JSON and exit.",
    )
    parser.add_argument(
        "--continue-on-failure",
        action="store_true",
        help="Keep running later smoke subgroups after a subgroup failure and report all collected failures.",
    )
    parser.add_argument(
        "--endpoint",
        help="Use an existing WebDriver HTTP endpoint instead of starting moli serve.",
    )
    parser.add_argument(
        "--browser-name",
        default="moli",
        help="browserName requested by Selenium. Defaults to moli.",
    )
    parser.add_argument(
        "--browser-binary",
        help="Browser binary used by ChromeDriver baselines.",
    )
    return parser.parse_args(argv)


async def async_main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.list_groups:
        print(json.dumps({"groups": group_listing()}, indent=2, ensure_ascii=False))
        return 0

    selection = resolve_group_selection(args.group)
    continue_on_failure = args.continue_on_failure or os.environ.get(
        "MOLI_WEBDRIVER_SMOKE_CONTINUE_ON_FAILURE"
    ) in {"1", "true", "yes"}
    fixture = FixtureServer()
    serve: MoliServe | None = None
    emit_serve_diagnostics = False
    results: list[dict[str, Any]] = []
    try:
        fixture.start()
        if args.endpoint:
            endpoint = args.endpoint.rstrip("/")
        else:
            if args.browser_name != "moli":
                raise RuntimeError(
                    "--browser-name must be moli when the runner starts moli serve"
                )
            port_env = os.environ.get("MOLI_WEBDRIVER_PORT")
            port = int(port_env) if port_env else reserve_port()
            if port <= 0 or port > 65535:
                raise RuntimeError(f"invalid MOLI_WEBDRIVER_PORT: {port_env}")
            serve = await start_moli_serve(port)
            endpoint = f"http://127.0.0.1:{port}"

        browser_binary = (
            Path(args.browser_binary).expanduser().resolve()
            if args.browser_binary
            else None
        )
        if browser_binary is not None and not browser_binary.is_file():
            raise RuntimeError(f"browser binary does not exist: {browser_binary}")
        target = WebDriverTarget(
            endpoint=endpoint,
            browser_name=args.browser_name,
            browser_binary=browser_binary,
        )
        await wait_for_webdriver_server(endpoint, serve)
        for group in selection:
            try:
                await group.runner(target, fixture.url, results, continue_on_failure)
            except Exception as error:
                if not continue_on_failure:
                    raise
                record_failure(results, group.name, f"{group.name}_group", error)
        ok = not has_failures(results)
        emit_serve_diagnostics = not ok
        print(
            json.dumps(
                {
                    "ok": ok,
                    "endpoint": endpoint,
                    "browserName": target.browser_name,
                    "browserBinary": str(target.browser_binary) if target.browser_binary else None,
                    "fixture": fixture.url,
                    "groups": [group.name for group in selection],
                    "continueOnFailure": continue_on_failure,
                    "results": results,
                },
                indent=2,
                ensure_ascii=False,
            )
        )
        return 0 if ok else 1
    except Exception as error:
        emit_serve_diagnostics = True
        print(
            json.dumps(
                {"ok": False, "error": "".join(traceback.format_exception(error)), "results": results},
                indent=2,
                ensure_ascii=False,
            ),
            file=sys.stderr,
        )
        return 1
    finally:
        await stop_moli_serve(serve)
        if emit_serve_diagnostics and serve is not None:
            print(render_moli_serve_diagnostics(serve), file=sys.stderr)
        fixture.stop()


def main(argv: list[str] | None = None) -> None:
    raise SystemExit(asyncio.run(async_main(argv)))
