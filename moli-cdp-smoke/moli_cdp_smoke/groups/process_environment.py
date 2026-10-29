from __future__ import annotations


import asyncio
import contextlib
from typing import Any

from ..diagnostics import new_cdp_session
from ..assertions import SmokeError, assert_equal, record
from ..config import is_moli_browser


READ_DEFAULTS = """() => [
  new Intl.NumberFormat().resolvedOptions().locale,
  new Intl.DateTimeFormat().resolvedOptions().timeZone,
  new Date('2024-01-01T00:00:00Z').getTimezoneOffset(),
  new Date('2024-07-01T00:00:00Z').getTimezoneOffset()
]"""


async def _expect_error(session: Any, method: str, params: dict[str, Any]) -> None:
    try:
        await session.send(method, params)
    except Exception as error:
        if "already in effect" in str(error):
            return
        raise SmokeError(f"{method} returned an unexpected error: {error}") from error
    raise SmokeError(f"{method} unexpectedly accepted a conflicting process claim")


async def _worker_defaults(page: Any) -> list[Any]:
    # A request/reply boundary, not a sleep or retry that could hide a lost wake.
    return await asyncio.wait_for(page.evaluate("""async () => {
      const read = channel => new Promise((resolve, reject) => {
        channel.onmessage = event => resolve(event.data);
        channel.onmessageerror = reject;
        channel.postMessage('read');
      });
      return [await read(globalThis.environmentWorker),
              await read(globalThis.environmentShared.port)];
    }"""), timeout=10)


async def _install_workers(page: Any) -> None:
    await page.evaluate("""read => {
      const source = 'const read = ' + read + ';' +
        'self.onmessage = () => postMessage(read());' +
        'self.onconnect = event => {' +
        ' const port = event.ports[0];' +
        ' port.onmessage = () => port.postMessage(read()); port.start(); };';
      const url = URL.createObjectURL(new Blob([source], {type: 'text/javascript'}));
      globalThis.environmentWorker = new Worker(url);
      globalThis.environmentShared = new SharedWorker(url, 'environment-defaults');
      environmentShared.port.start();
      globalThis.environmentDate = Date;
      globalThis.environmentIntl = Intl.DateTimeFormat;
      globalThis.environmentExistingDate = new Date('2024-01-01T00:00:00Z');
      environmentExistingDate.getTimezoneOffset();
    }""", READ_DEFAULTS)


async def _burst_environment_updates(control: Any, peer: Any, baseline: list[Any]) -> None:
    # Send a burst without interleaved observations. This is the latest-value
    # cache invalidation contract, not a requirement to replay every ICU value.
    updates = []
    for index in range(24):
        locale, timezone = ("fr_FR", "Europe/Paris") if index % 2 == 0 else (
            "de_DE", "Asia/Shanghai"
        )
        updates.extend([
            control.send("Emulation.setLocaleOverride", {"locale": locale}),
            control.send("Emulation.setTimezoneOverride", {"timezoneId": timezone}),
        ])
    await asyncio.wait_for(asyncio.gather(*updates), timeout=10)
    expected = ["de-DE", "Asia/Shanghai", -480, -480] if is_moli_browser() else baseline
    assert_equal(await peer.evaluate(READ_DEFAULTS), expected, "peer observes final burst defaults")
    assert_equal(await _worker_defaults(peer), [expected, expected], "workers observe final burst defaults")


async def _override_while_peer_paused(control: Any, peer: Any, baseline: list[Any]) -> None:
    # A paused isolate cannot drain its ordinary foreground notification. The
    # explicit owner notification must run before the next nested Inspector
    # command, without a generation check or resuming page JavaScript.
    await peer.send("Debugger.enable")
    paused = asyncio.get_running_loop().create_future()

    def on_paused(event: dict[str, Any]) -> None:
        if not paused.done():
            paused.set_result(event)

    peer.on("Debugger.paused", on_paused)
    evaluation = asyncio.create_task(peer.send("Runtime.evaluate", {
        "expression": "debugger;",
    }))

    async def cleanup_command(method: str) -> None:
        await asyncio.wait_for(peer.send(method), timeout=5)

    async def resume() -> None:
        if paused.done() and not paused.cancelled():
            await cleanup_command("Debugger.resume")

    async def finish_evaluation() -> None:
        await asyncio.wait_for(evaluation, timeout=5)

    async with contextlib.AsyncExitStack() as cleanup:
        # Every cleanup must run even if an earlier step fails. AsyncExitStack
        # preserves those failures through exception chaining.
        cleanup.push_async_callback(cleanup_command, "Debugger.disable")
        cleanup.push_async_callback(finish_evaluation)
        cleanup.push_async_callback(resume)
        cleanup.callback(peer.remove_listener, "Debugger.paused", on_paused)
        event = await asyncio.wait_for(paused, timeout=10)
        transitions = [
            ("fr_FR", "Europe/Paris", ["fr-FR", "Europe/Paris", -60, -120]),
            ("de_DE", "Asia/Shanghai", ["de-DE", "Asia/Shanghai", -480, -480]),
            ("", "", baseline),
            ("fr_FR", "Europe/Paris", ["fr-FR", "Europe/Paris", -60, -120]),
        ]
        for locale, timezone, expected in transitions:
            await control.send("Emulation.setLocaleOverride", {"locale": locale})
            await control.send("Emulation.setTimezoneOverride", {"timezoneId": timezone})
            if not is_moli_browser():
                owner = await control.send("Runtime.evaluate", {
                    "expression": f"({READ_DEFAULTS})()", "returnByValue": True,
                })
                assert_equal(owner.get("result", {}).get("value"), expected, "owner observes its override")
                expected = baseline
            # No sleep, retry, resume, or unrelated command to drive delivery.
            result = await peer.send("Debugger.evaluateOnCallFrame", {
                "callFrameId": event["callFrames"][0]["callFrameId"],
                "expression": f"[({READ_DEFAULTS})(), environmentExistingDate.getTimezoneOffset()]",
                "returnByValue": True,
            })
            assert_equal(result.get("result", {}).get("value"),
                         [expected, expected[2]],
                         f"paused isolate and existing Date observe {locale}/{timezone}")


async def run_process_environment_group(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    """Moli's one-process contract (Chrome can split these Pages into processes).

    Configuration ownership is session-local; ICU defaults are process-wide.
    Test cross-context propagation, late isolates, both Worker kinds, navigation,
    failed/same-value claims, detach, target/context close, and exact restoration.
    """
    owner_context = await browser.new_context()
    peer_context = await browser.new_context()
    sessions: list[Any] = []
    try:
        owner = await owner_context.new_page()
        peer = await peer_context.new_page()
        await owner.goto(f"{fixture}/plain")
        await peer.goto(f"{fixture}/plain")
        control = await new_cdp_session(owner_context, owner)
        other = await new_cdp_session(peer_context, peer)
        sessions.extend([control, other])
        baseline = await peer.evaluate(READ_DEFAULTS)
        await _install_workers(peer)
        assert_equal(await _worker_defaults(peer), [baseline, baseline], "worker baseline")
        record(results, "process_environment_workers_before_override")

        await _burst_environment_updates(control, peer, baseline)
        record(results, "process_environment_burst_updates_reach_pages_and_workers")
        await _override_while_peer_paused(control, other, baseline)
        record(results, "process_environment_paused_change_replace_restore")
        owner_expected = ["fr-FR", "Europe/Paris", -60, -120]
        expected = owner_expected if is_moli_browser() else baseline
        assert_equal(await owner.evaluate(READ_DEFAULTS), owner_expected, "owner override after paused peer")
        assert_equal(await peer.evaluate(READ_DEFAULTS), expected, "existing peer isolate")
        assert_equal(await _worker_defaults(peer), [expected, expected], "existing worker isolates")
        assert_equal(await peer.evaluate(
            "() => [Date === environmentDate, Intl.DateTimeFormat === environmentIntl]"
        ), [True, True], "native builtins are not replaced")
        record(results, "process_environment_existing_pages_and_workers")

        late = await peer_context.new_page()
        await late.goto(f"{fixture}/plain")
        assert_equal(await late.evaluate(READ_DEFAULTS), expected, "late isolate inherits ICU")
        await _install_workers(late)
        assert_equal(await _worker_defaults(late), [expected, expected], "late workers inherit ICU")
        await owner.reload()
        assert_equal(await owner.evaluate(READ_DEFAULTS), owner_expected, "navigation keeps session claim")
        record(results, "process_environment_new_isolate_and_navigation")

        if is_moli_browser():
            await _expect_error(other, "Emulation.setLocaleOverride", {"locale": "de_DE"})
            await _expect_error(other, "Emulation.setLocaleOverride", {"locale": ""})
            await _expect_error(other, "Emulation.setTimezoneOverride", {"timezoneId": "Asia/Shanghai"})
            await other.send("Emulation.setTimezoneOverride", {"timezoneId": "Europe/Paris"})
            await other.send("Emulation.setTimezoneOverride", {"timezoneId": ""})
        else:
            await other.send("Emulation.setLocaleOverride", {"locale": "de_DE"})
            await other.send("Emulation.setTimezoneOverride", {"timezoneId": "Asia/Shanghai"})
            assert_equal(await peer.evaluate(READ_DEFAULTS), ["de-DE", "Asia/Shanghai", -480, -480], "independent peer override")
            assert_equal(await owner.evaluate(READ_DEFAULTS), owner_expected, "peer override does not change owner")
            await other.send("Emulation.setLocaleOverride", {"locale": ""})
            await other.send("Emulation.setTimezoneOverride", {"timezoneId": ""})
        assert_equal(await peer.evaluate(READ_DEFAULTS), expected, "non-owner cannot clear")
        record(results, "process_environment_cross_target_exclusive_claims")

        await control.detach()
        sessions.remove(control)
        assert_equal(await owner.evaluate(READ_DEFAULTS), baseline, "detach restores owner defaults")
        assert_equal(await peer.evaluate(READ_DEFAULTS), baseline, "detach restores host defaults")
        assert_equal(await _worker_defaults(peer), [baseline, baseline], "worker restoration")
        assert_equal(await _worker_defaults(late), [baseline, baseline], "late worker restoration")
        await late.reload()
        assert_equal(await late.evaluate(READ_DEFAULTS), baseline, "no stale navigation replay")
        record(results, "process_environment_detach_restores_all_isolates")

        control = await new_cdp_session(owner_context, owner)
        sessions.append(control)
        await control.send("Emulation.setLocaleOverride", {"locale": "de_DE"})
        await control.send("Emulation.setTimezoneOverride", {"timezoneId": "Asia/Shanghai"})
        owner_expected = ["de-DE", "Asia/Shanghai", -480, -480]
        expected = owner_expected if is_moli_browser() else baseline
        assert_equal(await owner.evaluate(READ_DEFAULTS), owner_expected, "new owner override")
        assert_equal(await peer.evaluate(READ_DEFAULTS), expected, "new owner after release")
        await owner.close()
        sessions.remove(control)
        assert_equal(await peer.evaluate(READ_DEFAULTS), baseline, "target close releases claims")
        assert_equal(await _worker_defaults(peer), [baseline, baseline], "close restores workers")
        record(results, "process_environment_target_close_releases_claims")

        replacement = await owner_context.new_page()
        control = await new_cdp_session(owner_context, replacement)
        sessions.append(control)
        await control.send("Emulation.setLocaleOverride", {"locale": "de_DE"})
        await control.send("Emulation.setTimezoneOverride", {"timezoneId": "Asia/Shanghai"})
        assert_equal(await replacement.evaluate(READ_DEFAULTS), owner_expected, "replacement owner override")
        assert_equal(await peer.evaluate(READ_DEFAULTS), expected, "context owns live claims")
        await owner_context.close()
        sessions.remove(control)
        assert_equal(await peer.evaluate(READ_DEFAULTS), baseline, "context disposal releases claims")
        assert_equal(await _worker_defaults(peer), [baseline, baseline], "context disposal restores workers")
        record(results, "process_environment_context_disposal_releases_claims")

        await other.send("Emulation.setLocaleOverride", {"locale": "fr_FR"})
        await other.send("Emulation.setTimezoneOverride", {"timezoneId": "Europe/Paris"})
        await other.send("Emulation.setLocaleOverride", {"locale": ""})
        await other.send("Emulation.setTimezoneOverride", {"timezoneId": ""})
        assert_equal(await late.evaluate(READ_DEFAULTS), baseline, "explicit clear restores peers")
        record(results, "process_environment_explicit_clear")
    finally:
        for session in sessions:
            with contextlib.suppress(Exception):
                await session.detach()
        await owner_context.close()
        await peer_context.close()
