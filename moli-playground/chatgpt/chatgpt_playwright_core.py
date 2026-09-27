from __future__ import annotations

import asyncio
import getpass
import inspect
import json
import re
import sys
import time
from pathlib import Path
from argparse import Namespace
from typing import Any, Callable

from chatgpt_cdp_demo import (
    AnswerResult,
    CHATGPT_HELPER_JS,
    DemoError,
    MoliServe,
    read_json_url_no_proxy,
    redact_sensitive_text,
    redact_snapshot,
    start_moli,
    stop_moli,
)

from chatgpt_playwright_diagnostics import (
    redact_diagnostic_text,
    is_diagnostic_url,
    is_conversation_diagnostic_url,
    set_cookie_names_from_header,
    redacted_id_shape,
    chat_request_message_shape,
    chat_request_post_data_shape,
    summarize_live_trace,
    compact_live_trace_router_state,
    compact_live_trace_app_state_event,
    compact_live_trace_state_event,
    compact_live_trace_state,
    compact_live_trace_app_state,
    compact_live_trace_dom_node,
    compact_live_trace_dom_record,
    compact_live_trace_dom_sample,
    compact_live_trace_dom_samples,
    compact_live_trace_dom_mutations,
    compact_live_trace_value_shape,
    compact_live_trace_thread_renderer_probes,
    compact_live_trace_thread_renderer_probe,
    compact_live_trace_suspense_boundary_probes,
    compact_live_trace_suspense_boundary,
    compact_live_trace_suspense_internal,
    compact_live_trace_suspense_alternate,
    compact_live_trace_react_element_tree,
    compact_live_trace_react_element_type,
    compact_live_trace_fiber_internal_field,
    compact_live_trace_suspense_child,
    compact_live_trace_suspense_child_props,
    compact_live_trace_thread_store_detail,
    compact_live_trace_reference_shape,
    compact_live_trace_collection_entry,
    compact_live_trace_property_shape,
    compact_live_trace_function_result_shape,
    compact_live_trace_store_like_summary,
    compact_live_trace_result_detail,
    compact_live_trace_react_sample,
    compact_live_trace_conversation_wrapper,
    compact_live_trace_conversation_subtree,
    compact_live_trace_conversation_subtree_node,
    compact_live_trace_hook_state,
    compact_live_trace_thread_store_hooks,
    compact_live_trace_thread_store_hook_snapshot,
    compact_live_trace_reaction_store_probes,
    compact_live_trace_reaction_store_probe,
    compact_live_trace_reaction_store_detail,
    compact_live_trace_conversation_wrappers,
    compact_live_trace_thread_fiber,
    compact_live_trace_react_samples,
    compact_live_trace_react_commits,
)

Reporter = Callable[[str], None] | None
AnswerUpdate = Callable[[str], None] | None
WAITABLE_BLOCKING_REASONS = {"device-approval"}
CODE_BLOCKING_REASONS = {"email-verification", "verification-code"}

CHATGPT_LIVE_TRACE_JS = Path(__file__).with_name("chatgpt_playwright_live_trace.js").read_text(encoding="utf-8")


def looks_like_navigation_context_loss(error: BaseException) -> bool:
    text = str(error) or repr(error)
    return (
        "Execution context was destroyed" in text
        or "Cannot find context with specified id" in text
        or "Most likely because of a navigation" in text
    )


def looks_like_submit_timeout(error: BaseException) -> bool:
    text = str(error) or repr(error)
    return "timed out" in text and (
        "fillEmail" in text or "fillPassword" in text or "Page.evaluate" in text
    )


def auth_blocking_reason_from_url(url: str) -> str:
    lowered = url.lower()
    if "auth.openai.com/email-verification" in lowered:
        return "email-verification"
    return ""


def is_auth_password_url(url: str) -> bool:
    return "auth.openai.com/log-in/password" in url.lower()


def is_auth_password_missing_session_state(state: dict[str, Any]) -> bool:
    if state.get("hasPasswordInput"):
        return False
    text = str(state.get("text") or "").lower()
    return (
        "oops, an error occurred" in text
        and (
            "missing already entered username" in text
            or "unknown error" in text
            or "is_missing_session" in text
        )
    )


def is_actionable_auth_password_state(state: dict[str, Any]) -> bool:
    return (
        is_auth_password_url(str(state.get("url") or ""))
        and bool(state.get("hasPasswordInput"))
        and not bool(state.get("blockingReason"))
        and not bool(state.get("loggedIn"))
    )


def is_retryable_auth_password_error_state(state: dict[str, Any]) -> bool:
    if not is_auth_password_url(str(state.get("url") or "")):
        return False
    if state.get("hasPasswordInput") or state.get("loggedIn") or state.get("blockingReason"):
        return False
    text = str(state.get("text") or "").lower()
    return "try again" in text and (
        "operation timed out" in text
        or "oops, an error occurred" in text
        or "something went wrong" in text
    )


def is_waitable_blocking_reason(reason: Any) -> bool:
    return str(reason or "") in WAITABLE_BLOCKING_REASONS


def is_code_blocking_reason(reason: Any) -> bool:
    return str(reason or "") in CODE_BLOCKING_REASONS


def waitable_blocking_error(state: dict[str, Any]) -> DemoError:
    reason = str(state.get("blockingReason") or "auth-step")
    if reason == "device-approval":
        return DemoError(
            "login timed out while waiting for device approval; approve it on the listed device "
            f"or rerun with a longer --login-timeout; state={redact_snapshot(state)!r}"
        )
    return DemoError(f"login is waiting for auth step ({reason}); state={redact_snapshot(state)!r}")


def check_auth_intermediate_url(url: str) -> None:
    reason = auth_blocking_reason_from_url(url)
    if reason:
        raise DemoError(f"login reached an auth verification step ({reason}); this demo does not bypass it")


def is_transient_assistant_text(text: str) -> bool:
    normalized = re.sub(r"\s+", " ", text).strip().lower()
    return (
        normalized in {"", "thinking", "thinking...", "generating", "searching"}
        or normalized.endswith(" thinking")
        or normalized.endswith(" thinking...")
    )


def is_retryable_read_only_helper_error(error: BaseException) -> bool:
    text = str(error) or repr(error)
    return (
        "Playwright helper `" in text
        and ("timed out" in text or "Execution context was destroyed" in text)
    )





def suppress_known_navigation_context_loss(loop: asyncio.AbstractEventLoop) -> None:
    previous_handler = loop.get_exception_handler()

    def handler(current_loop: asyncio.AbstractEventLoop, context: dict[str, Any]) -> None:
        exception = context.get("exception")
        if isinstance(exception, BaseException) and looks_like_navigation_context_loss(exception):
            return
        if previous_handler is not None:
            previous_handler(current_loop, context)
        else:
            current_loop.default_exception_handler(context)

    loop.set_exception_handler(handler)


class PlaywrightChatGPTSession:
    def __init__(self, args: Namespace, reporter: Reporter = print) -> None:
        self.args = args
        self.reporter = reporter
        self.serve: MoliServe | None = None
        self.playwright: Any = None
        self.browser: Any = None
        self.context: Any = None
        self.page: Any = None
        self._close_context = False
        self.console_tail: list[str] = []
        self.page_error_tail: list[str] = []
        self.request_failed_tail: list[str] = []
        self.request_tail: list[str] = []
        self.response_tail: list[str] = []
        self.conversation_network_tail: list[str] = []
        self._auth_code_cache: str | None = None

    def report(self, message: str) -> None:
        if self.reporter is not None:
            self.reporter(message)

    def live_trace_enabled(self) -> bool:
        return bool(
            getattr(self.args, "live_trace", False)
            or getattr(self.args, "live_trace_output", "")
        )

    async def start(self) -> None:
        suppress_known_navigation_context_loss(asyncio.get_running_loop())
        try:
            from playwright.async_api import async_playwright
        except ImportError as error:
            raise DemoError(
                "missing dependency: playwright; run with "
                "`uv run --with websockets --with playwright python chatgpt_playwright_tui.py`"
            ) from error

        self.playwright = await async_playwright().start()
        backend = str(getattr(self.args, "backend", "moli") or "moli")
        if backend == "chromium":
            await self.start_chromium_backend()
        elif backend == "moli":
            await self.start_moli_backend()
        else:
            raise DemoError(f"unsupported Playwright backend: {backend!r}")

    async def start_moli_backend(self) -> None:
        if self.playwright is None:
            raise DemoError("Playwright is not initialized")
        self.report("starting moli serve")
        self.args.quiet = True
        self.serve = start_moli(self.args)
        self.report(f"connected CDP at {self.serve.endpoint}")
        version = await asyncio.to_thread(read_json_url_no_proxy, self.serve.endpoint.rstrip("/") + "/json/version")
        websocket_url = version.get("webSocketDebuggerUrl")
        if not isinstance(websocket_url, str) or not websocket_url:
            raise DemoError(f"missing webSocketDebuggerUrl in CDP version payload: {version!r}")
        self.browser = await self.playwright.chromium.connect_over_cdp(websocket_url)
        self.context = self.browser.contexts[0] if self.browser.contexts else await self.browser.new_context()
        await self.install_live_trace_init_script()
        await self.finish_page_setup()

    async def start_chromium_backend(self) -> None:
        if self.playwright is None:
            raise DemoError("Playwright is not initialized")
        launch_options: dict[str, Any] = {
            "headless": not bool(getattr(self.args, "headful", False)),
        }
        chromium_bin = str(getattr(self.args, "chromium_bin", "") or "")
        if chromium_bin:
            launch_options["executable_path"] = chromium_bin
        proxy = str(getattr(self.args, "http_proxy", "") or "")
        if proxy:
            launch_options["proxy"] = {"server": proxy}
            no_proxy = str(getattr(self.args, "http_no_proxy", "") or "")
            if no_proxy:
                launch_options["proxy"]["bypass"] = no_proxy
        context_options: dict[str, Any] = {}
        user_agent = str(getattr(self.args, "user_agent", "") or "")
        if user_agent:
            context_options["user_agent"] = user_agent
        profile_dir = str(getattr(self.args, "profile_dir", "") or "")
        if profile_dir:
            self.report("starting chromium persistent context")
            self.context = await self.playwright.chromium.launch_persistent_context(
                profile_dir,
                **launch_options,
                **context_options,
            )
        else:
            self.report("starting chromium")
            self.browser = await self.playwright.chromium.launch(**launch_options)
            self.context = await self.browser.new_context(**context_options)
        self._close_context = True
        await self.install_live_trace_init_script()
        await self.finish_page_setup()

    async def finish_page_setup(self) -> None:
        if self.context is None:
            raise DemoError("Playwright context is not initialized")
        self.page = self.context.pages[0] if self.context.pages else await self.context.new_page()
        self.page.set_default_timeout(10_000)
        self.page.on("console", self._record_console)
        self.page.on("pageerror", self._record_page_error)
        self.page.on("request", self._record_request)
        self.page.on("response", self._record_response)
        self.page.on("requestfailed", self._record_request_failed)
        self.page.on(
            "framenavigated",
            lambda frame: self.report(f"frame navigated: {redact_diagnostic_text(frame.url)}")
            if frame == self.page.main_frame
            else None,
        )

    async def install_live_trace_init_script(self) -> None:
        if not self.live_trace_enabled() or self.context is None:
            return
        self.report("install live trace")
        await self.context.add_init_script(CHATGPT_LIVE_TRACE_JS)

    def _append_tail(self, target: list[str], text: str, *, limit: int = 20) -> None:
        target.append(text)
        if len(target) > limit:
            del target[: len(target) - limit]

    def _record_console(self, message: Any) -> None:
        location = getattr(message, "location", {}) or {}
        location_text = ""
        if isinstance(location, dict) and location.get("url"):
            location_text = (
                f" @{redact_diagnostic_text(str(location.get('url') or ''))}:"
                f"{location.get('lineNumber', '')}:{location.get('columnNumber', '')}"
            )
        text = f"{getattr(message, 'type', '')}: {getattr(message, 'text', '')}{location_text}"
        self._append_tail(self.console_tail, text[:1000])
        args = list(getattr(message, "args", []) or [])
        if args and getattr(message, "type", "") in {"error", "warning"}:
            try:
                loop = asyncio.get_running_loop()
                loop.create_task(self._record_console_arg_details(args, location_text))
            except RuntimeError:
                pass

    async def _record_console_arg_details(self, args: list[Any], location_text: str) -> None:
        details: list[str] = []
        for handle in args[:4]:
            try:
                value = await handle.evaluate(
                    """value => {
                      if (value instanceof Error) {
                        return {
                          kind: 'Error',
                          name: value.name,
                          message: value.message,
                          stack: value.stack,
                          cause: value.cause && String(value.cause)
                        };
                      }
                      if (value && typeof value === 'object') {
                        return {
                          kind: Object.prototype.toString.call(value),
                          name: value.name,
                          message: value.message,
                          code: value.code,
                          stack: value.stack,
                          text: String(value)
                        };
                      }
                      return {kind: typeof value, text: String(value)};
                    }"""
                )
            except Exception as error:
                details.append(f"<arg-error {type(error).__name__}: {error}>")
                continue
            details.append(redact_diagnostic_text(repr(value))[:700])
        if details:
            self._append_tail(
                self.console_tail,
                f"console-args{location_text}: {' | '.join(details)}"[:2000],
            )

    def _record_page_error(self, error: BaseException) -> None:
        self._append_tail(self.page_error_tail, (str(error) or repr(error))[:1000])

    def _record_request_failed(self, request: Any) -> None:
        failure = request.failure
        detail = failure if isinstance(failure, str) else str(failure)
        self._append_tail(
            self.request_failed_tail,
            redact_diagnostic_text(f"{request.method} {request.url} {detail}"[:1000]),
        )
        if is_conversation_diagnostic_url(request.url):
            self._append_tail(
                self.conversation_network_tail,
                redact_diagnostic_text(f"FAILED {request.method} {request.resource_type} {request.url} {detail}"[:1000]),
                limit=300,
            )

    def _record_request(self, request: Any) -> None:
        if not is_diagnostic_url(request.url):
            return
        message = redact_diagnostic_text(f"{request.method} {request.resource_type} {request.url}"[:1000])
        self._append_tail(self.request_tail, message, limit=200)
        if is_conversation_diagnostic_url(request.url):
            self._append_tail(self.conversation_network_tail, f"REQ {message}", limit=300)
            if "chatgpt.com/backend-api/f/conversation" in request.url:
                try:
                    post_data = getattr(request, "post_data", None)
                    if callable(post_data):
                        post_data = post_data()
                except Exception:
                    post_data = None
                shape = chat_request_post_data_shape(post_data if isinstance(post_data, str) else None)
                if shape is not None:
                    self._append_tail(
                        self.conversation_network_tail,
                        f"REQ_BODY_SHAPE {redact_snapshot(shape)!r}",
                        limit=300,
                    )

    def _record_response(self, response: Any) -> None:
        url = response.url
        if not is_diagnostic_url(url):
            return
        headers = getattr(response, "headers", {}) or {}
        details: list[str] = []
        set_cookie = headers.get("set-cookie") or headers.get("Set-Cookie")
        if isinstance(set_cookie, str) and set_cookie:
            names = set_cookie_names_from_header(set_cookie)
            details.append(f"set-cookie={','.join(names) if names else '<present>'}")
        location = headers.get("location") or headers.get("Location")
        if isinstance(location, str) and location:
            details.append(f"location={redact_diagnostic_text(location)}")
        suffix = f" [{' '.join(details)}]" if details else ""
        message = redact_diagnostic_text(f"{response.status} {url}{suffix}"[:1000])
        self._append_tail(self.response_tail, message, limit=200)
        if is_conversation_diagnostic_url(url):
            self._append_tail(self.conversation_network_tail, f"RES {message}", limit=300)

    def diagnostic_tail(self) -> dict[str, list[str]]:
        return {
            "console": self.console_tail[-10:],
            "page_errors": self.page_error_tail[-10:],
            "requests": self.request_tail[-60:],
            "responses": self.response_tail[-60:],
            "conversation_network": self.conversation_network_tail[-120:],
            "request_failed": self.request_failed_tail[-10:],
        }

    async def live_trace_snapshot_best_effort(self, *, timeout: float = 5.0) -> dict[str, Any] | None:
        if not self.live_trace_enabled() or self.page is None:
            return None
        try:
            trace = await asyncio.wait_for(
                self.page.evaluate(
                    """() => {
                      if (typeof window.__lmChatGPTLiveTraceSnapshot !== 'function') return null;
                      return window.__lmChatGPTLiveTraceSnapshot();
                    }"""
                ),
                timeout=timeout,
            )
        except Exception as error:  # noqa: BLE001 - diagnostics should not hide the primary failure.
            return {"error": type(error).__name__, "message": str(error)}
        return trace if isinstance(trace, dict) else None

    async def write_live_trace_summary(
        self,
        reason: str,
        summary: dict[str, Any] | None = None,
    ) -> None:
        output = str(getattr(self.args, "live_trace_output", "") or "")
        if not output:
            return
        try:
            if summary is None:
                summary = summarize_live_trace(await self.live_trace_snapshot_best_effort(timeout=5))
            payload = redact_snapshot(
                {
                    "reason": reason,
                    "timestamp": time.time(),
                    "url": self.page.url if self.page is not None else "",
                    "trace": summary,
                }
            )
            with open(output, "a", encoding="utf-8") as handle:
                handle.write(json.dumps(payload, ensure_ascii=False, sort_keys=True))
                handle.write("\n")
            self.report(f"wrote live trace summary: {output}")
        except Exception as error:  # noqa: BLE001 - diagnostics should not hide the primary result.
            self.report(f"failed to write live trace summary: {type(error).__name__}: {error}")

    async def close(self) -> None:
        if self.context is not None and self._close_context:
            try:
                await self.context.close()
            except Exception:
                pass
            self.context = None
        if self.browser is not None:
            try:
                await self.browser.close()
            except Exception:
                pass
            self.browser = None
        if self.playwright is not None:
            try:
                await self.playwright.stop()
            except Exception:
                pass
            self.playwright = None
        if self.serve is not None:
            stop_moli(self.serve)
            self.serve = None

    async def install_helpers(self) -> None:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        if await self.page.evaluate("!!window.__lmChatGPTDemo"):
            return
        await self.page.evaluate(CHATGPT_HELPER_JS)

    async def wait_for_document_settle(self, *, timeout: float = 5.0) -> None:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        try:
            await self.page.wait_for_load_state("domcontentloaded", timeout=int(timeout * 1000))
        except Exception:
            pass
        await asyncio.sleep(0.25)

    async def helper(self, name: str, *args: Any, timeout: float = 10.0) -> Any:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        read_only_helper = name in {"loginState", "conversationState", "latestAssistantText", "snapshot"}
        attempts = 2 if read_only_helper else 1
        async def invoke() -> Any:
            await self.install_helpers()
            return await self.page.evaluate(
                """([name, args]) => window.__lmChatGPTDemo[name](...args)""",
                [name, list(args)],
            )

        for attempt in range(attempts):
            try:
                return await asyncio.wait_for(invoke(), timeout=timeout)
            except TimeoutError as error:
                if read_only_helper and attempt + 1 < attempts:
                    await self.wait_for_document_settle(timeout=min(5.0, timeout))
                    continue
                raise DemoError(
                    f"Playwright helper `{name}` timed out after {timeout:.1f}s; "
                    f"diagnostics={self.diagnostic_tail()!r}"
                ) from error
            except Exception as error:
                if read_only_helper and attempt + 1 < attempts and looks_like_navigation_context_loss(error):
                    await self.wait_for_document_settle(timeout=min(5.0, timeout))
                    continue
                text = str(error) or repr(error)
                raise DemoError(f"Playwright helper `{name}` failed: {text}") from error
        raise DemoError(f"Playwright helper `{name}` failed")

    async def conversation_state_best_effort(self, *, timeout: float = 10.0) -> dict[str, Any]:
        try:
            state = await self.helper("conversationState", timeout=timeout)
        except DemoError as error:
            if is_retryable_read_only_helper_error(error):
                return {}
            raise
        return state if isinstance(state, dict) else {}

    async def wait_for_state(self, predicate: Callable[[dict[str, Any]], bool], *, timeout: float, label: str) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        last_state: dict[str, Any] | None = None
        last_error: BaseException | None = None
        reported_waitable_reasons: set[str] = set()
        while time.monotonic() < deadline:
            try:
                state = await self.helper("loginState", timeout=15)
                if isinstance(state, dict):
                    last_state = state
                    if predicate(state):
                        return state
                    blocking_reason = state.get("blockingReason")
                    if blocking_reason:
                        if is_waitable_blocking_reason(blocking_reason):
                            reason = str(blocking_reason)
                            if reason not in reported_waitable_reasons:
                                self.report(f"waiting for auth step: {reason}")
                                reported_waitable_reasons.add(reason)
                            await asyncio.sleep(1.0)
                            continue
                        return state
            except BaseException as error:  # noqa: BLE001 - preserve final context.
                last_error = error
            await asyncio.sleep(0.5)
        if isinstance(last_state, dict) and is_waitable_blocking_reason(last_state.get("blockingReason")):
            raise waitable_blocking_error(last_state)
        raise DemoError(
            f"timed out waiting for {label}; "
            f"last_state={last_state!r}; last_error={last_error!r}; diagnostics={self.diagnostic_tail()!r}"
        )

    async def wait_for_login_form_hydration(self, *, timeout: float) -> dict[str, Any]:
        try:
            return await self.wait_for_state(
                lambda value: bool(
                    value.get("loggedIn")
                    or value.get("hasPasswordInput")
                    or not value.get("hasEmailInput")
                    or value.get("loginFormHydrated")
                ),
                timeout=timeout,
                label="hydrated login form",
            )
        except DemoError as error:
            self.report(f"continue before hydration: {error}")
            state = await self.helper("loginState", timeout=15)
            return state if isinstance(state, dict) else {}

    async def wait_for_password_input_on_auth_page(self, *, timeout: float) -> dict[str, Any]:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        deadline = time.monotonic() + timeout
        selector = 'input[type="password"], input[name="password"], #password'
        last_error: BaseException | None = None
        reloaded_after_missing_session = False
        while time.monotonic() < deadline:
            url = self.page.url
            check_auth_intermediate_url(url)
            if is_auth_password_url(url):
                try:
                    await self.page.locator(selector).first.wait_for(state="attached", timeout=2000)
                    return {
                        "loggedIn": False,
                        "url": self.page.url,
                        "hasEmailInput": False,
                        "hasPasswordInput": True,
                        "blockingReason": "",
                    }
                except BaseException as error:  # noqa: BLE001 - keep polling through auth page hydration.
                    last_error = error
                if not reloaded_after_missing_session:
                    try:
                        state = await self.helper("snapshot", timeout=5)
                    except BaseException as error:  # noqa: BLE001 - keep original wait error too.
                        last_error = error
                    else:
                        if isinstance(state, dict) and is_auth_password_missing_session_state(state):
                            self.report("reload auth password page after session cookie")
                            reloaded_after_missing_session = True
                            await self.page.reload(
                                wait_until="domcontentloaded",
                                timeout=int(max(5.0, deadline - time.monotonic()) * 1000),
                            )
                            continue
            else:
                try:
                    await self.page.wait_for_url("**/log-in/password", timeout=1000)
                except BaseException as error:  # noqa: BLE001 - report final context below.
                    last_error = error
            await asyncio.sleep(0.25)
        raise DemoError(
            "timed out waiting for auth password page; "
            f"url={self.page.url!r}; last_error={last_error!r}; diagnostics={self.diagnostic_tail()!r}"
        )

    async def wait_for_auth_password_runtime_ready(self, *, timeout: float) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        last_state: dict[str, Any] | None = None
        while time.monotonic() < deadline:
            state = await self.helper("loginState", timeout=10)
            if not isinstance(state, dict):
                await asyncio.sleep(0.5)
                continue
            last_state = state
            if not is_actionable_auth_password_state(state):
                return state
            cookie_names = set(state.get("cookieNames") or [])
            if {"iss_context", "rg_context"}.issubset(cookie_names):
                return state
            await asyncio.sleep(0.5)
        if isinstance(last_state, dict):
            self.report(f"continue before auth runtime cookies ready: cookies={last_state.get('cookieNames')!r}")
            return last_state
        return {}

    async def wait_for_state_after_email_submit(self, *, timeout: float, label: str) -> dict[str, Any]:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        deadline = time.monotonic() + timeout
        last_state: dict[str, Any] | None = None
        last_error: BaseException | None = None
        while time.monotonic() < deadline:
            url = self.page.url
            check_auth_intermediate_url(url)
            if is_auth_password_url(url):
                return await self.wait_for_password_input_on_auth_page(
                    timeout=max(1.0, deadline - time.monotonic()),
                )
            try:
                state = await self.helper("loginState", timeout=5)
                if isinstance(state, dict):
                    last_state = state
                    check_human_gate(state)
                    if state.get("hasPasswordInput") or state.get("loggedIn"):
                        return state
            except BaseException as error:  # noqa: BLE001 - navigation can invalidate Runtime.evaluate.
                last_error = error
            await asyncio.sleep(0.5)
        raise DemoError(
            f"timed out waiting for {label}; "
            f"url={self.page.url!r}; last_state={last_state!r}; "
            f"last_error={last_error!r}; diagnostics={self.diagnostic_tail()!r}"
        )

    def email_submit_compat_error(self, state: dict[str, Any], original_error: BaseException) -> DemoError:
        url = str(state.get("url") or "")
        text = str(state.get("text") or "")
        return DemoError(
            "ChatGPT email submit did not advance to the password page under Moli. "
            "The page stayed on the SSR email form instead of running the auth frontend redirect "
            "to `https://auth.openai.com/log-in/password`. "
            f"url={url!r}; hasEmailInput={state.get('hasEmailInput')!r}; "
            f"hasPasswordInput={state.get('hasPasswordInput')!r}; "
            f"text={text[:240]!r}; first_error={original_error}; diagnostics={self.diagnostic_tail()!r}"
        )

    async def accept_cookie_consent(self) -> bool:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        for label in ("Accept all", "Reject non-essential"):
            try:
                await self.page.get_by_text(label, exact=True).click(timeout=2000)
                self.report(f"cookie consent: {label}")
                return True
            except Exception:
                continue
        try:
            result = await self.helper("acceptCookieConsent", timeout=3)
            if isinstance(result, dict) and result.get("ok"):
                self.report(f"cookie consent: {result.get('text') or 'accepted'}")
                return True
        except Exception:
            pass
        return False

    async def click_login_native(self) -> None:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        selectors = [
            '[data-testid="login-button"]',
            'a[href*="/auth/login"]',
            'button:has-text("Log in")',
            'a:has-text("Log in")',
            'text=Log in',
        ]
        for selector in selectors:
            try:
                await self.page.locator(selector).first.click(timeout=3000)
                return
            except Exception:
                continue
        await self.page.goto(
            self.absolute_url("/auth/login"),
            wait_until="domcontentloaded",
            timeout=int(min(30.0, self.args.login_timeout) * 1000),
        )

    async def wait_for_login_entry_quick(self, *, timeout: float) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        last_state: dict[str, Any] | None = None
        while time.monotonic() < deadline:
            try:
                state = await self.helper("loginState", timeout=5)
            except DemoError:
                await asyncio.sleep(0.25)
                continue
            if isinstance(state, dict):
                last_state = state
                if state.get("hasEmailInput") or state.get("hasPasswordInput") or state.get("loggedIn"):
                    return state
                if state.get("blockingReason"):
                    return state
            await asyncio.sleep(0.5)
        return last_state or {}

    async def navigate_to_login_form(self) -> dict[str, Any]:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        self.report("open login form")
        await self.click_login_native()
        await self.accept_cookie_consent()
        state = await self.wait_for_login_entry_quick(timeout=min(6.0, self.args.login_timeout))
        if state.get("hasEmailInput") or state.get("hasPasswordInput") or state.get("loggedIn"):
            return state

        if state.get("hasLoginButton"):
            self.report("retry login after cookie consent")
            await self.click_login_native()
            state = await self.wait_for_login_entry_quick(timeout=min(6.0, self.args.login_timeout))
            if state.get("hasEmailInput") or state.get("hasPasswordInput") or state.get("loggedIn"):
                return state

        self.report("navigate directly to /auth/login")
        await self.page.goto(
            self.absolute_url("/auth/login"),
            wait_until="domcontentloaded",
            timeout=int(min(30.0, self.args.login_timeout) * 1000),
        )
        await self.accept_cookie_consent()
        return await self.wait_for_state(
            lambda value: bool(value.get("hasEmailInput") or value.get("hasPasswordInput") or value.get("loggedIn")),
            timeout=min(30.0, self.args.login_timeout),
            label="login form",
        )

    async def fill_email_native(self, email: str) -> None:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        await self.page.locator('input[type="email"], input[name="email"], #email').first.fill(email, timeout=10_000)
        last_error: BaseException | None = None
        try:
            await self.page.locator("button").filter(has_text=re.compile(r"^Continue$")).first.click(timeout=5000)
            return
        except Exception as error:
            last_error = error
        for selector in ('input[type="submit"]',):
            try:
                await self.page.locator(selector).filter(has_text=re.compile(r"^Continue$")).first.click(timeout=5000)
                return
            except Exception as error:
                last_error = error
        try:
            await self.page.locator('input[type="email"], input[name="email"], #email').first.press("Enter", timeout=3000)
            return
        except Exception as error:
            last_error = error
        raise DemoError(f"failed to submit email form with Playwright locators: {last_error!r}")

    async def fill_password_native(self, password: str) -> None:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        await self.page.locator('input[type="password"], input[name="password"], #password').first.fill(
            password,
            timeout=10_000,
        )
        last_error: BaseException | None = None
        for label in ("Log in", "Continue"):
            try:
                await self.page.locator("button").filter(has_text=re.compile(rf"^{re.escape(label)}$")).first.click(
                    timeout=5000,
                )
                return
            except Exception as error:
                last_error = error
        try:
            await self.page.locator('input[type="password"], input[name="password"], #password').first.press(
                "Enter",
                timeout=3000,
            )
            return
        except Exception as error:
            last_error = error
        raise DemoError(f"failed to submit password form with Playwright locators: {last_error!r}")

    async def fill_prompt_native(self, prompt: str) -> dict[str, Any]:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        selectors = [
            'div.ProseMirror[contenteditable="true"]',
            '[contenteditable="plaintext-only"]',
            '[contenteditable="true"][id="prompt-textarea"]',
            '[contenteditable="true"][data-testid*="prompt" i]',
            'form [contenteditable="true"]',
            'textarea[name="prompt-textarea"]',
            'textarea[data-testid*="prompt" i]',
            'textarea[placeholder*="Message" i]',
            '#prompt-textarea',
        ]
        last_error: BaseException | None = None
        for selector in selectors:
            locator = self.page.locator(selector).first
            try:
                await locator.wait_for(state="visible", timeout=2500)
            except Exception as error:
                last_error = error
                continue
            try:
                await locator.fill(prompt, timeout=5000)
                await asyncio.sleep(0.1)
                state = await self.helper("conversationState", timeout=5)
                if isinstance(state, dict) and state.get("sendUsable"):
                    return {"ok": True, "method": "locator.fill", "selector": selector, "state": state}
            except Exception as error:
                last_error = error

            try:
                await locator.click(timeout=5000)
                await self.page.keyboard.press("Control+A")
                await self.page.keyboard.press("Backspace")
                await self.page.keyboard.insert_text(prompt)
                await asyncio.sleep(0.1)
                state = await self.helper("conversationState", timeout=5)
                if isinstance(state, dict) and state.get("sendUsable"):
                    return {"ok": True, "method": "keyboard.insert_text", "selector": selector, "state": state}
                return {
                    "ok": False,
                    "reason": "send-button-not-usable-after-native-input",
                    "selector": selector,
                    "state": state,
                }
            except Exception as error:
                last_error = error
                continue
        raise DemoError(f"failed to fill prompt with Playwright native input: {last_error!r}")

    async def click_send_prompt_native(self) -> dict[str, Any]:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        selectors = [
            'button[data-testid="send-button"]',
            'button[aria-label*="Send" i]',
            'form button[type="submit"]',
            'button:has-text("Send")',
        ]
        last_error: BaseException | None = None
        for selector in selectors:
            locator = self.page.locator(selector).first
            try:
                await locator.click(timeout=5000)
                return {"ok": True, "method": "locator.click", "selector": selector}
            except Exception as error:
                last_error = error
            try:
                await locator.click(timeout=3000, force=True)
                return {"ok": True, "method": "locator.click(force)", "selector": selector}
            except Exception as error:
                last_error = error
        raise DemoError(f"failed to click send button with Playwright locators: {last_error!r}")

    async def fill_password_with_helper_after_native_error(
        self,
        password: str,
        native_error: BaseException,
        *,
        timeout: float,
    ) -> None:
        try:
            await self.helper("fillPassword", password, timeout=timeout)
        except DemoError as helper_error:
            if looks_like_navigation_context_loss(helper_error):
                self.report("password submit triggered navigation")
            elif looks_like_submit_timeout(helper_error):
                self.report("password submit timed out; checking page state")
            else:
                raise helper_error from native_error

    async def maybe_retry_password_submit(self, password: str, *, timeout: float) -> None:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        try:
            state = await self.helper("loginState", timeout=10)
        except DemoError as state_error:
            if looks_like_navigation_context_loss(state_error):
                self.report("password submit still navigating")
                return
            raise
        if not isinstance(state, dict) or not is_actionable_auth_password_state(state):
            if isinstance(state, dict) and is_waitable_blocking_reason(state.get("blockingReason")):
                self.report(f"waiting for auth step: {state.get('blockingReason')}")
                return
            check_human_gate(state if isinstance(state, dict) else {})
            return
        self.report("retry password submit with page helper")
        try:
            await self.helper("fillPassword", password, timeout=timeout)
        except DemoError as helper_error:
            if looks_like_navigation_context_loss(helper_error):
                self.report("password submit triggered navigation")
            elif looks_like_submit_timeout(helper_error):
                self.report("password submit timed out; checking page state")
            else:
                raise

    async def click_auth_try_again(self) -> bool:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        candidates = [
            self.page.get_by_role("button", name=re.compile(r"^try again$", re.IGNORECASE)),
            self.page.locator("button, a").filter(has_text=re.compile(r"^Try again$", re.IGNORECASE)),
            self.page.get_by_text(re.compile(r"^Try again$", re.IGNORECASE)),
        ]
        for candidate in candidates:
            try:
                await candidate.first.click(timeout=5000)
                self.report("retry auth password page after transient error")
                await self.wait_for_document_settle(timeout=5)
                return True
            except Exception:
                continue
        return False

    async def click_login_with_one_time_code(self) -> bool:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        pattern = re.compile(r"(one[- ]time code|email code|log in with.*code)", re.IGNORECASE)
        candidates = [
            self.page.get_by_role("button", name=pattern),
            self.page.get_by_role("link", name=pattern),
            self.page.locator("button, a").filter(has_text=pattern),
            self.page.get_by_text(pattern),
        ]
        for candidate in candidates:
            try:
                await candidate.first.click(timeout=5000)
                self.report("switch auth to one-time code")
                await self.wait_for_document_settle(timeout=5)
                return True
            except Exception:
                continue
        return False

    def has_auth_code_source(self) -> bool:
        provider = getattr(self.args, "auth_code_provider", None)
        return bool(
            str(getattr(self.args, "auth_code", "") or "")
            or bool(getattr(self.args, "auth_code_stdin", False))
            or bool(getattr(self.args, "auth_code_prompt", False))
            or callable(provider)
        )

    async def read_auth_code(self) -> str:
        if self._auth_code_cache is not None:
            return self._auth_code_cache
        raw = str(getattr(self.args, "auth_code", "") or "")
        if not raw and bool(getattr(self.args, "auth_code_stdin", False)):
            self.report("read auth code from stdin")
            raw = await asyncio.to_thread(sys.stdin.readline)
        if not raw and bool(getattr(self.args, "auth_code_prompt", False)):
            if not sys.stdin.isatty():
                raise DemoError("--auth-code-prompt requires an interactive terminal")
            raw = await asyncio.to_thread(getpass.getpass, "ChatGPT auth code: ")
        if not raw:
            provider = getattr(self.args, "auth_code_provider", None)
            if callable(provider):
                self.report("wait for auth code input")
                raw = provider()
                if inspect.isawaitable(raw):
                    raw = await raw
        code = re.sub(r"[\s-]+", "", raw)
        if not code:
            raise DemoError("auth code is required; pass --auth-code, --auth-code-stdin, or --auth-code-prompt")
        self._auth_code_cache = code
        return code

    async def click_try_with_email_verification(self) -> bool:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        candidates = [
            self.page.get_by_role("button", name=re.compile(r"try with email", re.IGNORECASE)),
            self.page.get_by_text(re.compile(r"try with email", re.IGNORECASE)),
            self.page.locator("button, a").filter(has_text=re.compile(r"try with email", re.IGNORECASE)),
        ]
        for candidate in candidates:
            try:
                await candidate.first.click(timeout=5000)
                self.report("switch auth to email verification")
                await self.wait_for_document_settle(timeout=5)
                return True
            except Exception:
                continue
        return False

    async def fill_auth_code_native(self, code: str) -> None:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        selectors = [
            'input[autocomplete="one-time-code"]',
            'input[inputmode="numeric"]',
            'input[name*="code" i]',
            'input[id*="code" i]',
            'input[type="tel"]',
            'input[type="text"]',
        ]
        inputs: list[Any] = []
        for selector in selectors:
            locator = self.page.locator(selector)
            try:
                count = await locator.count()
            except Exception:
                continue
            for index in range(min(count, 12)):
                candidate = locator.nth(index)
                try:
                    if await candidate.is_visible(timeout=1000) and await candidate.is_enabled(timeout=1000):
                        inputs.append(candidate)
                except Exception:
                    continue
            if inputs:
                break
        if not inputs:
            state = await self.helper("snapshot", timeout=10)
            raise DemoError(f"auth code input not found; state={redact_snapshot(state)!r}")
        if len(inputs) > 1 and len(code) >= len(inputs):
            for index, candidate in enumerate(inputs):
                await candidate.fill(code[index], timeout=5000)
        else:
            await inputs[0].fill(code, timeout=10_000)
        last_error: BaseException | None = None
        for label in ("Continue", "Verify", "Submit", "Next"):
            try:
                await self.page.locator("button").filter(has_text=re.compile(rf"^{re.escape(label)}$")).first.click(
                    timeout=5000,
                )
                return
            except Exception as error:
                last_error = error
        try:
            await inputs[-1].press("Enter", timeout=3000)
            return
        except Exception as error:
            last_error = error
        raise DemoError(f"failed to submit auth code form with Playwright locators: {last_error!r}")

    async def wait_for_login_completion_after_password(self, password: str, *, timeout: float) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        started = time.monotonic()
        last_state: dict[str, Any] | None = None
        last_error: BaseException | None = None
        reported_reasons: set[str] = set()
        retried_password_submit = False
        tried_email_verification = False
        auth_code_attempts = 0
        auth_code_submitted_at: float | None = None
        reported_auth_code_wait = False
        last_progress_report = 0.0
        auth_error_retries = 0
        while time.monotonic() < deadline:
            try:
                state = await self.helper("loginState", timeout=15)
            except BaseException as error:  # noqa: BLE001 - navigation can invalidate Runtime.evaluate.
                last_error = error
                now = time.monotonic()
                if now - last_progress_report >= 15.0:
                    url = self.page.url if self.page is not None else ""
                    self.report(f"waiting for login state: url={redact_diagnostic_text(url)} error={error}")
                    last_progress_report = now
                await asyncio.sleep(0.5)
                continue
            if not isinstance(state, dict):
                await asyncio.sleep(0.5)
                continue
            last_state = state
            if state.get("loggedIn"):
                return state
            if (
                not retried_password_submit
                and time.monotonic() - started >= 8.0
                and is_actionable_auth_password_state(state)
            ):
                await self.maybe_retry_password_submit(password, timeout=45)
                retried_password_submit = True
                await asyncio.sleep(1.0)
                continue
            if is_retryable_auth_password_error_state(state) and auth_error_retries < 2:
                auth_error_retries += 1
                if await self.click_auth_try_again():
                    try:
                        retry_state = await self.wait_for_password_input_on_auth_page(timeout=20)
                    except DemoError:
                        await asyncio.sleep(1.0)
                        continue
                    if retry_state.get("hasPasswordInput"):
                        if bool(getattr(self.args, "try_email_verification", False)) and not tried_email_verification:
                            tried_email_verification = True
                            if await self.click_login_with_one_time_code():
                                await asyncio.sleep(1.0)
                                continue
                        await self.wait_for_auth_password_runtime_ready(timeout=15)
                        self.report("fill password after auth retry")
                        await self.helper("fillPassword", password, timeout=45)
                        await asyncio.sleep(1.5)
                        continue
            reason = state.get("blockingReason") or auth_blocking_reason_from_url(str(state.get("url") or ""))
            if reason == "device-approval":
                if bool(getattr(self.args, "try_email_verification", False)) and not tried_email_verification:
                    tried_email_verification = True
                    if await self.click_try_with_email_verification():
                        await asyncio.sleep(1.0)
                        continue
                if reason not in reported_reasons:
                    self.report("waiting for auth step: device-approval")
                    reported_reasons.add(str(reason))
                await asyncio.sleep(1.0)
                continue
            if is_code_blocking_reason(reason):
                now = time.monotonic()
                can_retry_code = bool(
                    getattr(self.args, "auth_code_stdin", False)
                    or getattr(self.args, "auth_code_prompt", False)
                    or callable(getattr(self.args, "auth_code_provider", None))
                )
                waiting_after_submit = (
                    auth_code_submitted_at is not None
                    and now - auth_code_submitted_at < 30.0
                )
                if auth_code_submitted_at is not None and waiting_after_submit:
                    if not reported_auth_code_wait:
                        self.report("waiting for auth code verification")
                        reported_auth_code_wait = True
                    await asyncio.sleep(1.0)
                    continue
                if auth_code_attempts > 0 and not can_retry_code:
                    raise DemoError(
                        "login stayed on the auth code step after submitting a code; "
                        f"state={redact_snapshot(state)!r}; diagnostics={self.diagnostic_tail()!r}"
                    )
                if auth_code_attempts > 0:
                    self._auth_code_cache = None
                    self.report("auth code still pending; read another auth code")
                if self.has_auth_code_source():
                    code = await self.read_auth_code()
                    self.report("fill auth code")
                    await self.fill_auth_code_native(code)
                    auth_code_attempts += 1
                    auth_code_submitted_at = time.monotonic()
                    reported_auth_code_wait = False
                    if bool(getattr(self.args, "debug_snapshot", False)):
                        try:
                            snapshot = await self.helper("snapshot", timeout=10)
                        except DemoError:
                            snapshot = {}
                        self.report(f"auth code submit state: {redact_snapshot(snapshot)!r}")
                    await asyncio.sleep(1.5)
                    continue
                raise DemoError(
                    f"login reached an auth code step ({reason}); "
                    "rerun with --auth-code, --auth-code-stdin, or --auth-code-prompt"
                )
            if reason:
                raise DemoError(f"login reached a verification step ({reason}); this demo does not bypass it")
            now = time.monotonic()
            if now - last_progress_report >= 15.0:
                text = redact_diagnostic_text(str(state.get("text") or "").replace("\n", " ")[:240])
                self.report(
                    "waiting for logged-in composer: "
                    f"url={redact_diagnostic_text(str(state.get('url') or ''))} "
                    f"hasPasswordInput={bool(state.get('hasPasswordInput'))} "
                    f"hasComposer={bool(state.get('hasComposer'))} "
                    f"hasLoginButton={bool(state.get('hasLoginButton'))} "
                    f"hasAccountCookie={bool(state.get('hasAccountCookie'))} "
                    f"cookies={state.get('cookieNames')!r} "
                    f"lastPasswordSubmit={redact_snapshot(state.get('lastPasswordSubmit'))!r} "
                    f"text={text!r}"
                )
                last_progress_report = now
            await asyncio.sleep(0.5)
        if isinstance(last_state, dict) and is_waitable_blocking_reason(last_state.get("blockingReason")):
            raise waitable_blocking_error(last_state)
        raise DemoError(
            "timed out waiting for logged-in ChatGPT composer after password submit; "
            f"last_state={redact_snapshot(last_state)!r}; "
            f"last_error={last_error!r}; diagnostics={self.diagnostic_tail()!r}"
        )

    def absolute_url(self, path: str) -> str:
        if path.startswith("http://") or path.startswith("https://"):
            return path
        return self.args.url.rstrip("/") + "/" + path.lstrip("/")

    async def navigate_to_initial_login_state(self) -> dict[str, Any]:
        if self.page is None:
            raise DemoError("Playwright page is not initialized")
        self.report(f"navigate: {self.args.url}")
        await self.page.goto(
            self.args.url,
            wait_until="domcontentloaded",
            timeout=int(self.args.login_timeout * 1000),
        )
        await self.accept_cookie_consent()
        return await self.helper("loginState", timeout=10)

    async def require_existing_session(self) -> None:
        state = await self.navigate_to_initial_login_state()
        if isinstance(state, dict) and state.get("loggedIn"):
            self.report("already logged in")
            return
        raise DemoError(
            "existing profile is not logged in; "
            "rerun with --email and --password-stdin or use a logged-in --profile-dir; "
            f"state={redact_snapshot(state)!r}"
        )

    async def login(self, email: str, password: str) -> None:
        state = await self.navigate_to_initial_login_state()
        if isinstance(state, dict) and state.get("loggedIn"):
            self.report("already logged in")
            return

        state = await self.navigate_to_login_form()
        check_human_gate(state)
        if state.get("loggedIn"):
            return

        if state.get("hasEmailInput"):
            if not state.get("loginFormHydrated"):
                self.report("wait for hydrated login form")
                state = await self.wait_for_login_form_hydration(
                    timeout=min(35.0, self.args.login_timeout),
                )
                check_human_gate(state)
                if state.get("loggedIn"):
                    return
                if state.get("hasPasswordInput"):
                    pass
                elif not state.get("hasEmailInput"):
                    state = await self.helper("loginState", timeout=15)

        if state.get("hasEmailInput"):
            self.report("fill email")
            await self.accept_cookie_consent()
            if state.get("loginFormHydrated"):
                try:
                    await self.helper("fillEmail", email, timeout=45)
                except DemoError as helper_error:
                    if looks_like_navigation_context_loss(helper_error):
                        self.report("email submit triggered navigation")
                    elif looks_like_submit_timeout(helper_error):
                        self.report("email submit timed out; checking page state")
                    else:
                        await self.fill_email_native(email)
            else:
                try:
                    await self.fill_email_native(email)
                except Exception as native_error:
                    try:
                        await self.helper("fillEmail", email, timeout=45)
                    except DemoError as helper_error:
                        if looks_like_navigation_context_loss(helper_error):
                            self.report("email submit triggered navigation")
                        elif looks_like_submit_timeout(helper_error):
                            self.report("email submit timed out; checking page state")
                        else:
                            raise helper_error from native_error
            await asyncio.sleep(1.5)
            try:
                state = await self.wait_for_state_after_email_submit(
                    timeout=min(45.0, self.args.login_timeout),
                    label="password form after native email submit",
                )
            except DemoError as first_error:
                check_auth_intermediate_url(self.page.url if self.page is not None else "")
                self.report("retry email submit after cookie consent")
                await self.accept_cookie_consent()
                try:
                    await self.helper("fillEmail", email, timeout=45)
                except DemoError as helper_error:
                    if looks_like_navigation_context_loss(helper_error):
                        self.report("email submit triggered navigation")
                    elif looks_like_submit_timeout(helper_error):
                        self.report("email submit timed out; checking page state")
                    else:
                        raise helper_error from first_error
                await asyncio.sleep(1.5)
                try:
                    state = await self.wait_for_state_after_email_submit(
                        timeout=min(45.0, self.args.login_timeout),
                        label="password form",
                    )
                except DemoError:
                    check_auth_intermediate_url(self.page.url if self.page is not None else "")
                    try:
                        current = await self.helper("loginState", timeout=15)
                    except DemoError as state_error:
                        current = {
                            "url": self.page.url if self.page is not None else "",
                            "stateError": str(state_error),
                        }
                    if isinstance(current, dict):
                        raise self.email_submit_compat_error(current, first_error) from first_error
                    raise
            check_human_gate(state)

        if state.get("loggedIn"):
            return
        if not state.get("hasPasswordInput"):
            raise DemoError(f"password input did not appear; state={redact_snapshot(state)!r}")

        await self.wait_for_auth_password_runtime_ready(timeout=15)
        self.report("fill password")
        try:
            await self.helper("fillPassword", password, timeout=45)
        except DemoError as helper_error:
            if looks_like_navigation_context_loss(helper_error):
                self.report("password submit triggered navigation")
            elif looks_like_submit_timeout(helper_error):
                self.report("password submit timed out; checking page state")
            else:
                try:
                    await self.fill_password_native(password)
                except Exception as native_error:
                    raise helper_error from native_error
        await asyncio.sleep(1.5)
        state = await self.wait_for_login_completion_after_password(
            password,
            timeout=max(1.0, self.args.login_timeout),
        )
        check_human_gate(state)
        if not state.get("loggedIn"):
            raise DemoError(f"login did not reach composer; state={redact_snapshot(state)!r}")

    def reload_recovery_enabled(self) -> bool:
        return not bool(getattr(self.args, "no_reload_recovery", False))

    async def ask_once(self, prompt: str, answer_update: AnswerUpdate = None) -> AnswerResult:
        before = await self.conversation_state_best_effort(timeout=10)
        self.report("send prompt")
        try:
            result = await self.fill_prompt_native(prompt)
            if not result.get("ok"):
                self.report(f"native prompt input incomplete: {result.get('reason') or 'unknown'}")
                raise DemoError(
                    f"native prompt input did not make send usable: {redact_snapshot(result)!r}"
                )
            self.report(f"prompt input: {result.get('method')}")
        except Exception as native_error:
            self.report("retry prompt input with page helper")
            result = await self.helper("fillPrompt", prompt, timeout=10)
            if not isinstance(result, dict) or not result.get("ok"):
                raise DemoError(f"failed to fill prompt: {redact_snapshot(result)!r}") from native_error
        send_deadline = time.monotonic() + min(30.0, max(5.0, self.args.answer_timeout / 3))
        send_state: dict[str, Any] = {}
        last_send_error: BaseException | None = None
        while time.monotonic() < send_deadline:
            try:
                state = await self.helper("conversationState", timeout=10)
            except DemoError as error:
                if not is_retryable_read_only_helper_error(error):
                    raise
                last_send_error = error
                await asyncio.sleep(0.5)
                continue
            except BaseException as error:  # noqa: BLE001 - preserve final diagnostics.
                last_send_error = error
                await asyncio.sleep(0.5)
                continue
            if isinstance(state, dict):
                send_state = state
                reason = state.get("blockingReason")
                if reason:
                    raise DemoError(
                        f"ChatGPT reached a blocking step before send ({reason}); "
                        f"state={redact_snapshot(state)!r}"
                    )
                if state.get("sendUsable"):
                    break
            await asyncio.sleep(0.5)
        else:
            raise DemoError(
                "timed out waiting for usable send button; "
                f"state={redact_snapshot(send_state)!r}; last_error={last_send_error!r}; "
                f"diagnostics={self.diagnostic_tail()!r}"
            )
        before_click_url = self.page.url if self.page is not None else ""
        try:
            result = await self.click_send_prompt_native()
            self.report(f"send click: {result.get('method')}")
        except Exception as native_error:
            if isinstance(native_error, DemoError) and looks_like_navigation_context_loss(native_error):
                self.report("send prompt triggered navigation")
                await self.wait_for_document_settle(timeout=5)
                result = {"ok": True, "method": "navigation-context-loss"}
            else:
                post_click_state = await self.conversation_state_best_effort(timeout=5)
                post_click_url = self.page.url if self.page is not None else ""
                click_appears_submitted = bool(
                    (post_click_url and post_click_url != before_click_url and "/c/" in post_click_url)
                    or post_click_state.get("isGenerating")
                    or post_click_state.get("stopButton")
                )
                if click_appears_submitted:
                    self.report("send click continued after native click error")
                    result = {"ok": True, "method": "locator.click(post-state)"}
                else:
                    self.report("retry send click with page helper")
                    try:
                        result = await self.helper("clickSendPrompt", timeout=10)
                    except DemoError as error:
                        if not looks_like_navigation_context_loss(error):
                            raise
                        self.report("send prompt triggered navigation")
                        await self.wait_for_document_settle(timeout=5)
                        result = {"ok": True, "method": "helper-navigation-context-loss"}
                    else:
                        if not isinstance(result, dict) or not result.get("ok"):
                            raise DemoError(
                                f"failed to click send button: result={redact_snapshot(result)!r}; "
                                f"state={redact_snapshot(send_state)!r}"
                            ) from native_error
        else:
            if not isinstance(result, dict) or not result.get("ok"):
                raise DemoError(
                    f"failed to click send button: result={redact_snapshot(result)!r}; "
                    f"state={redact_snapshot(send_state)!r}"
                )
        self.report("waiting for response")

        deadline = time.monotonic() + self.args.answer_timeout
        before_count = int(before.get("assistantCount") or 0)
        before_text = str(before.get("latestAssistantText") or "")
        last = ""
        last_error: BaseException | None = None
        reloaded_conversation = False
        reload_after = min(90.0, max(45.0, self.args.answer_timeout / 3))
        wait_started = time.monotonic()
        stable_since: float | None = None
        idle_no_render_since: float | None = None
        while time.monotonic() < deadline:
            try:
                state = await self.helper("conversationState", timeout=10)
            except DemoError as error:
                if not is_retryable_read_only_helper_error(error):
                    raise
                last_error = error
                await asyncio.sleep(1.0)
                continue
            if not isinstance(state, dict):
                state = {}
            reason = state.get("blockingReason")
            if reason:
                raise DemoError(f"ChatGPT reached a blocking step while waiting for answer ({reason})")
            current = str(state.get("latestAssistantText") or "")
            count = int(state.get("assistantCount") or 0)
            is_generating = bool(state.get("isGenerating"))
            if (
                current
                and not is_transient_assistant_text(current)
                and (count > before_count or current != before_text)
            ):
                if current != last:
                    last = current
                    stable_since = time.monotonic()
                    if answer_update is not None:
                        answer_update(current)
                elif stable_since is not None and time.monotonic() - stable_since >= 2.0 and not is_generating:
                    await self.write_live_trace_summary("live-dom")
                    return AnswerResult(text=current, source="live-dom")
            elif last and stable_since is not None and not is_generating and time.monotonic() - stable_since >= 2.0:
                await self.write_live_trace_summary("live-dom")
                return AnswerResult(text=last, source="live-dom")
            now = time.monotonic()
            if (
                self.reload_recovery_enabled()
                and not reloaded_conversation
                and not last
                and not current
                and not is_generating
                and count <= before_count
                and "/c/" in str(state.get("url") or "")
            ):
                if idle_no_render_since is None:
                    idle_no_render_since = now
                    self.report("conversation idle without response DOM; checking persisted response")
                elif now - idle_no_render_since >= 3.0:
                    reloaded_conversation = True
                    recovered = await self.reload_current_conversation_for_response(
                        before_count,
                        before_text,
                        timeout=max(1.0, min(90.0, deadline - now)),
                        answer_update=answer_update,
                    )
                    if recovered is not None:
                        return recovered
            elif (
                not self.reload_recovery_enabled()
                and not last
                and not current
                and not is_generating
                and count <= before_count
                and "/c/" in str(state.get("url") or "")
            ):
                if idle_no_render_since is None:
                    idle_no_render_since = now
                    self.report("conversation idle without response DOM; reload recovery disabled")
            else:
                idle_no_render_since = None
            if (
                self.reload_recovery_enabled()
                and not reloaded_conversation
                and not last
                and "/c/" in str(state.get("url") or "")
                and now - wait_started >= reload_after
            ):
                reloaded_conversation = True
                recovered = await self.reload_current_conversation_for_response(
                    before_count,
                    before_text,
                    timeout=max(1.0, min(90.0, deadline - now)),
                    answer_update=answer_update,
                )
                if recovered is not None:
                    return recovered
            await asyncio.sleep(1.0)
        try:
            final_state = await self.helper("conversationState", timeout=10)
        except DemoError as error:
            final_state = {}
            last_error = error
        live_trace = summarize_live_trace(await self.live_trace_snapshot_best_effort(timeout=5))
        await self.write_live_trace_summary("timeout", live_trace)
        raise DemoError(
            "timed out waiting for ChatGPT response; "
            f"last={last[-1000:]!r}; state={redact_snapshot(final_state)!r}; "
            f"reload_recovery={self.reload_recovery_enabled()}; "
            f"live_trace={redact_snapshot(live_trace)!r}; "
            f"last_error={last_error!r}; diagnostics={self.diagnostic_tail()!r}"
        )

    async def reload_current_conversation_for_response(
        self,
        before_count: int,
        before_text: str,
        *,
        timeout: float,
        answer_update: AnswerUpdate = None,
    ) -> AnswerResult | None:
        if self.page is None:
            return None
        url = str(self.page.url or "")
        if "/c/" not in url:
            return None
        self.report("reload conversation to verify persisted response")
        try:
            await self.page.goto(url, wait_until="domcontentloaded", timeout=30_000)
        except Exception as error:
            if not looks_like_navigation_context_loss(error):
                detail = redact_diagnostic_text(str(error))
                self.report(f"conversation reload did not finish cleanly: {detail}")
        deadline = time.monotonic() + timeout
        latest = ""
        stable_since: float | None = None
        while time.monotonic() < deadline:
            try:
                state = await self.helper("conversationState", timeout=15)
            except DemoError as error:
                if not is_retryable_read_only_helper_error(error):
                    raise
                await asyncio.sleep(1.0)
                continue
            if not isinstance(state, dict):
                state = {}
            reason = state.get("blockingReason")
            if reason:
                raise DemoError(
                    f"ChatGPT reached a blocking step after conversation reload ({reason})"
                )
            current = str(state.get("latestAssistantText") or "")
            count = int(state.get("assistantCount") or 0)
            is_generating = bool(state.get("isGenerating"))
            if (
                current
                and not is_transient_assistant_text(current)
                and (count > before_count or current != before_text)
            ):
                if current != latest:
                    latest = current
                    stable_since = time.monotonic()
                    if answer_update is not None:
                        answer_update(current)
                elif stable_since is not None and time.monotonic() - stable_since >= 2.0 and not is_generating:
                    await self.write_live_trace_summary("persisted-reload")
                    return AnswerResult(text=current, source="persisted-reload")
            elif latest and stable_since is not None and not is_generating and time.monotonic() - stable_since >= 2.0:
                await self.write_live_trace_summary("persisted-reload")
                return AnswerResult(text=latest, source="persisted-reload")
            await asyncio.sleep(1.0)
        return None


def check_human_gate(state: dict[str, Any]) -> None:
    reason = state.get("blockingReason") or auth_blocking_reason_from_url(str(state.get("url") or ""))
    if reason:
        raise DemoError(f"login reached a verification step ({reason}); this demo does not bypass it")
