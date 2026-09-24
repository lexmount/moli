from __future__ import annotations

import asyncio
import contextlib
import json
from dataclasses import dataclass, field
from typing import Any, Callable

from ..assertions import SmokeError, assert_equal, record_contract
from ..raw_cdp import RawCdpClient, connect_raw_cdp


OBSERVE_EXPRESSION = r"""
(() => JSON.stringify({
  url: location.href,
  dom_text: (document.body?.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 1600),
  visible_text: (document.body?.innerText || '').replace(/\s+/g, ' ').trim().slice(0, 1600),
  elements: Array.from(document.querySelectorAll(
    'input,textarea,select,button,a,[role=button],[contenteditable=true]'
  )).filter(el => {
    const r = el.getBoundingClientRect();
    return r.width > 1 && r.height > 1;
  }).slice(0, 32).map((el, i) => {
    const id = `lex-${i}`;
    el.setAttribute('data-lex-id', id);
    return {selector: `[data-lex-id=${id}]`};
  }),
  realm: {
    old_marker_type: typeof globalThis.__agentOldRealmMarker,
    new_marker: globalThis.__agentNewRealmMarker ?? null
  }
}))()
"""


@dataclass
class _Transcript:
    messages: list[dict[str, Any]] = field(default_factory=list)
    response_ids: list[int] = field(default_factory=list)
    evaluate_ids: list[int] = field(default_factory=list)

    def append(self, message: dict[str, Any]) -> None:
        self.messages.append(message)
        response_id = message.get("id")
        if isinstance(response_id, int):
            self.response_ids.append(response_id)


async def run_agent_episode_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    client = await connect_raw_cdp(endpoint)
    transcript = _Transcript()
    browser_context_id: str | None = None
    session_id: str | None = None
    try:
        context = await _command(client, transcript, "Target.createBrowserContext")
        browser_context_id = _required_string(
            context.get("result", {}),
            "browserContextId",
        )
        created = await _command(
            client,
            transcript,
            "Target.createTarget",
            {"browserContextId": browser_context_id, "url": "about:blank"},
        )
        target_id = _required_string(created.get("result", {}), "targetId")
        attached = await _command(
            client,
            transcript,
            "Target.attachToTarget",
            {"targetId": target_id, "flatten": True},
        )
        session_id = _required_string(attached.get("result", {}), "sessionId")
        await _command(client, transcript, "Page.enable", session_id=session_id)
        await _command(client, transcript, "Runtime.enable", session_id=session_id)

        source_url = f"{fixture}/agent-episode-smoke"
        await _navigate_and_wait(client, transcript, session_id, source_url)
        await _command(client, transcript, "Page.captureScreenshot", session_id=session_id)
        initial = await _observe(client, transcript, session_id)
        assert_equal(initial["url"], source_url, "agent episode initial URL")
        assert_equal(len(initial["elements"]), 2, "agent episode initial control count")

        await _observe(client, transcript, session_id)
        fill_value = await _evaluate(
            client,
            transcript,
            session_id,
            """(() => { const e=document.querySelector('[data-lex-id=lex-0]');
              if(!e) throw new Error('selector not found'); e.focus();
              const p=Object.getPrototypeOf(e);
              const d=Object.getOwnPropertyDescriptor(p,'value');
              if(d&&d.set) d.set.call(e,'moli'); else e.value='moli';
              e.dispatchEvent(new InputEvent('input',{bubbles:true}));
              e.dispatchEvent(new Event('change',{bubbles:true}));
              return 'filled'; })()""",
        )
        assert_equal(fill_value, "filled", "agent episode fill response")

        filled = await _observe(client, transcript, session_id)
        if "value:moli;events:input,change" not in filled["dom_text"]:
            raise SmokeError(f"agent episode live DOM fill state was stale: {filled!r}")

        await _observe(client, transcript, session_id)
        click_start_index = len(transcript.messages)
        click_id = await client.send(
            "Runtime.evaluate",
            {
                "expression": """(() => { const e=document.querySelector('[data-lex-id=lex-1]');
                  if(!e) throw new Error('selector not found'); e.focus();
                  for(const t of ['pointerdown','mousedown','pointerup','mouseup'])
                    e.dispatchEvent(new MouseEvent(t,{bubbles:true,cancelable:true,view:window}));
                  e.click(); return 'clicked'; })()""",
                "returnByValue": True,
                "awaitPromise": True,
            },
            session_id=session_id,
        )
        transcript.evaluate_ids.append(click_id)
        click_response = await _receive_until(
            client,
            transcript,
            lambda message: message.get("id") == click_id,
            "agent episode click response",
        )
        _raise_cdp_error(click_response, "Runtime.evaluate click")
        assert_equal(
            click_response.get("result", {}).get("result", {}).get("value"),
            "clicked",
            "agent episode click response",
        )
        result_url = f"{fixture}/agent-episode-smoke-result?value=moli"
        await _receive_until(
            client,
            transcript,
            lambda message: (
                message.get("sessionId") == session_id
                and message.get("method") == "Page.loadEventFired"
                and _has_frame_navigation_after(
                    transcript.messages,
                    session_id,
                    result_url,
                    click_id,
                )
            ),
            "agent episode replacement load",
        )
        _assert_realm_replacement_order(
            transcript.messages,
            session_id,
            result_url,
            click_id,
            click_start_index,
        )

        replacement = await _observe(client, transcript, session_id)
        assert_equal(replacement["url"], result_url, "agent episode replacement URL")
        assert_equal(
            replacement["realm"],
            {"old_marker_type": "undefined", "new_marker": "result-realm"},
            "agent episode replacement realm",
        )
        if "agent episode result moli" not in replacement["visible_text"]:
            raise SmokeError(f"agent episode replacement observation was stale: {replacement!r}")

        failed_url = f"{fixture}/chromium-network-reset-before-response?agent-episode=1"
        failed_navigation = await _navigate_and_wait_for_error_document(
            client,
            transcript,
            session_id,
            failed_url,
        )
        if "ERR_CONNECTION_RESET" not in str(
            failed_navigation.get("result", {}).get("errorText", "")
        ):
            raise SmokeError(
                f"agent episode failed navigation returned no connection reset: {failed_navigation!r}"
            )
        error_observation = await _observe(client, transcript, session_id)
        assert_equal(
            error_observation["url"],
            "chrome-error://chromewebdata/",
            "agent episode error Document URL",
        )
        assert_equal(
            error_observation["realm"]["old_marker_type"],
            "undefined",
            "agent episode error Document old realm visibility",
        )
        frame_tree = await _command(
            client,
            transcript,
            "Page.getFrameTree",
            session_id=session_id,
        )
        frame = frame_tree.get("result", {}).get("frameTree", {}).get("frame", {})
        assert_equal(frame.get("unreachableUrl"), failed_url, "error Document unreachableUrl")

        await _command(client, transcript, "Browser.getVersion")
        _assert_single_evaluate_responses(transcript)
        _assert_forbidden_errors_absent(transcript.messages)
        record_contract(
            results,
            "agent_episode_exact_rl_cdp_path",
            contract=(
                "RL-shaped awaitPromise observe/fill/click survives realm replacement, "
                "then a failed navigation commits a usable error Document"
            ),
            source="recorded RL replay expressions plus Chromium CDP ordering",
            commands=[
                "Target.createBrowserContext",
                "Target.createTarget",
                "Target.attachToTarget",
                "Page.enable",
                "Runtime.enable",
                "Page.navigate",
                "Runtime.evaluate(awaitPromise=true)",
                "Page.getFrameTree",
            ],
            observed={
                "runtimeEvaluateCount": len(transcript.evaluate_ids),
                "messageCount": len(transcript.messages),
                "replacementUrl": replacement["url"],
                "errorDocumentUrl": error_observation["url"],
                "unreachableUrl": frame.get("unreachableUrl"),
                "filledVisibleTextFresh": (
                    "value:moli;events:input,change" in filled["visible_text"]
                ),
            },
        )
    finally:
        if browser_context_id is not None:
            with contextlib.suppress(Exception):
                await _command(
                    client,
                    transcript,
                    "Target.disposeBrowserContext",
                    {"browserContextId": browser_context_id},
                )
        await client.websocket.close()


async def _command(
    client: RawCdpClient,
    transcript: _Transcript,
    method: str,
    params: dict[str, Any] | None = None,
    *,
    session_id: str | None = None,
) -> dict[str, Any]:
    message_id = await client.send(method, params, session_id=session_id)
    response = await _receive_until(
        client,
        transcript,
        lambda message: message.get("id") == message_id,
        f"{method} response",
    )
    _raise_cdp_error(response, method)
    return response


async def _evaluate(
    client: RawCdpClient,
    transcript: _Transcript,
    session_id: str,
    expression: str,
) -> Any:
    message_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": expression,
            "returnByValue": True,
            "awaitPromise": True,
        },
        session_id=session_id,
    )
    transcript.evaluate_ids.append(message_id)
    response = await _receive_until(
        client,
        transcript,
        lambda message: message.get("id") == message_id,
        "Runtime.evaluate response",
    )
    _raise_cdp_error(response, "Runtime.evaluate")
    result = response.get("result", {})
    exception = result.get("exceptionDetails")
    if exception is not None:
        raise SmokeError(f"Runtime.evaluate JavaScript exception: {exception!r}")
    return result.get("result", {}).get("value")


async def _observe(
    client: RawCdpClient,
    transcript: _Transcript,
    session_id: str,
) -> dict[str, Any]:
    value = await _evaluate(
        client,
        transcript,
        session_id,
        OBSERVE_EXPRESSION,
    )
    if not isinstance(value, str):
        raise SmokeError(f"agent episode observation was not a string: {value!r}")
    observation = json.loads(value)
    if not isinstance(observation, dict):
        raise SmokeError(f"agent episode observation was not an object: {observation!r}")
    return observation


async def _navigate_and_wait(
    client: RawCdpClient,
    transcript: _Transcript,
    session_id: str,
    url: str,
) -> None:
    navigate_id = await client.send(
        "Page.navigate",
        {"url": url},
        session_id=session_id,
    )
    saw_response = False
    saw_load = False

    def complete(message: dict[str, Any]) -> bool:
        nonlocal saw_response, saw_load
        if message.get("id") == navigate_id:
            _raise_cdp_error(message, "Page.navigate")
            if message.get("result", {}).get("errorText"):
                raise SmokeError(f"Page.navigate failed: {message!r}")
            saw_response = True
        if message.get("sessionId") == session_id and message.get("method") == "Page.loadEventFired":
            saw_load = True
        return saw_response and saw_load and _has_frame_navigation(
            transcript.messages,
            session_id,
            url,
        )

    await _receive_until(client, transcript, complete, f"load for {url}")


async def _navigate_and_wait_for_error_document(
    client: RawCdpClient,
    transcript: _Transcript,
    session_id: str,
    url: str,
) -> dict[str, Any]:
    navigate_id = await client.send(
        "Page.navigate",
        {"url": url},
        session_id=session_id,
    )
    response: dict[str, Any] | None = None
    saw_load = False

    def complete(message: dict[str, Any]) -> bool:
        nonlocal response, saw_load
        if message.get("id") == navigate_id:
            _raise_cdp_error(message, "Page.navigate failed navigation")
            response = message
        if message.get("sessionId") == session_id and message.get("method") == "Page.loadEventFired":
            saw_load = True
        return response is not None and saw_load and any(
            item.get("sessionId") == session_id
            and item.get("method") == "Page.frameNavigated"
            and item.get("params", {}).get("frame", {}).get("url")
            == "chrome-error://chromewebdata/"
            and item.get("params", {}).get("frame", {}).get("unreachableUrl") == url
            for item in transcript.messages
        )

    await _receive_until(client, transcript, complete, "failed navigation error Document")
    if response is None:
        raise SmokeError("failed navigation produced no Page.navigate response")
    return response


async def _receive_until(
    client: RawCdpClient,
    transcript: _Transcript,
    predicate: Callable[[dict[str, Any]], bool],
    label: str,
    *,
    timeout: float = 10.0,
) -> dict[str, Any]:
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                f"timed out waiting for {label}; messages={transcript.messages[-30:]!r}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        transcript.append(message)
        if predicate(message):
            return message


def _raise_cdp_error(response: dict[str, Any], label: str) -> None:
    error = response.get("error")
    if error is not None:
        raise SmokeError(f"{label} returned CDP error: {error!r}")


def _required_string(value: dict[str, Any], key: str) -> str:
    result = value.get(key)
    if not isinstance(result, str) or not result:
        raise SmokeError(f"missing {key}: {value!r}")
    return result


def _has_frame_navigation(
    messages: list[dict[str, Any]],
    session_id: str,
    url: str,
) -> bool:
    return any(
        message.get("sessionId") == session_id
        and message.get("method") == "Page.frameNavigated"
        and message.get("params", {}).get("frame", {}).get("url") == url
        for message in messages
    )


def _has_frame_navigation_after(
    messages: list[dict[str, Any]],
    session_id: str,
    url: str,
    response_id: int,
) -> bool:
    response_index = next(
        (index for index, message in enumerate(messages) if message.get("id") == response_id),
        None,
    )
    if response_index is None:
        return False
    return _has_frame_navigation(messages[response_index + 1 :], session_id, url)


def _assert_realm_replacement_order(
    messages: list[dict[str, Any]],
    session_id: str,
    result_url: str,
    response_id: int,
    click_start_index: int,
) -> None:
    response_indexes = [
        index for index, message in enumerate(messages) if message.get("id") == response_id
    ]
    assert_equal(len(response_indexes), 1, "agent episode click terminal response count")
    response_index = response_indexes[0]

    def event_index(method: str, predicate: Callable[[dict[str, Any]], bool]) -> int:
        for index, message in enumerate(
            messages[click_start_index:],
            start=click_start_index,
        ):
            if (
                message.get("sessionId") == session_id
                and message.get("method") == method
                and predicate(message)
            ):
                return index
        raise SmokeError(f"missing {method} during agent episode replacement")

    cleared = event_index("Runtime.executionContextsCleared", lambda _message: True)
    started = event_index(
        "Page.frameStartedNavigating",
        lambda message: message.get("params", {}).get("url") == result_url,
    )
    navigated = event_index(
        "Page.frameNavigated",
        lambda message: message.get("params", {}).get("frame", {}).get("url") == result_url,
    )
    created = event_index(
        "Runtime.executionContextCreated",
        lambda message: bool(
            message.get("params", {})
            .get("context", {})
            .get("auxData", {})
            .get("isDefault")
        ),
    )
    if not all(response_index < index for index in (cleared, started, navigated)):
        raise SmokeError(
            "agent episode Runtime response did not precede destructive navigation events: "
            f"response={response_index}, cleared={cleared}, started={started}, navigated={navigated}"
        )
    if not cleared < created:
        raise SmokeError(
            f"agent episode old realm clear did not precede replacement realm: {cleared} >= {created}"
        )


def _assert_single_evaluate_responses(transcript: _Transcript) -> None:
    for message_id in transcript.evaluate_ids:
        count = transcript.response_ids.count(message_id)
        assert_equal(count, 1, f"Runtime.evaluate response count for id={message_id}")


def _assert_forbidden_errors_absent(messages: list[dict[str, Any]]) -> None:
    forbidden = {"Promise was collected", "NoDocumentLoaded"}
    observed = [
        message.get("error", {}).get("message")
        for message in messages
        if isinstance(message.get("error"), dict)
    ]
    found = [message for message in observed if message in forbidden]
    if found:
        raise SmokeError(f"agent episode received forbidden CDP errors: {found!r}")
