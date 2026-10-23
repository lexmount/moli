from __future__ import annotations

import asyncio
import sys

import pytest

from moli_frontend_smoke import browsers


@pytest.mark.parametrize("stage", ["announcement", "discovery"])
def test_cancelled_startup_reaps_child_and_removes_profile(monkeypatch, tmp_path, stage):
    async def scenario():
        profile = tmp_path / "browser-profile"
        profile.mkdir()
        reached_stage = asyncio.Event()
        processes = []
        original_create = asyncio.create_subprocess_exec
        original_announcement = browsers._wait_for_announced_endpoint

        async def record_process(*args, **kwargs):
            process = await original_create(*args, **kwargs)
            processes.append(process)
            return process

        async def wait_announcement(*args, **kwargs):
            if stage == "announcement":
                reached_stage.set()
            return await original_announcement(*args, **kwargs)

        async def wait_discovery(*args, **kwargs):
            reached_stage.set()
            await asyncio.Event().wait()

        monkeypatch.setattr(asyncio, "create_subprocess_exec", record_process)
        monkeypatch.setattr(browsers, "_wait_for_announced_endpoint", wait_announcement)
        monkeypatch.setattr(browsers, "wait_for_cdp_endpoint", wait_discovery)
        startup = asyncio.create_task(browsers._start_process(
            name="dummy",
            command=[sys.executable, "-u", "-c", "import time; print('ready'); time.sleep(60)"],
            endpoint_parser=lambda line: "http://127.0.0.1:1" if line == "ready" else None,
            temp_dirs=[str(profile)],
        ))
        try:
            await asyncio.wait_for(reached_stage.wait(), timeout=5)
            startup.cancel()
            with pytest.raises(asyncio.CancelledError):
                await startup
            assert processes[0].returncode is not None, "cancelled startup leaked its child"
            assert not profile.exists(), "cancelled startup leaked its profile"
        finally:
            if not startup.done():
                startup.cancel()
            await asyncio.gather(startup, return_exceptions=True)
            for process in processes:
                if process.returncode is None:
                    process.kill()
                await process.wait()

    asyncio.run(scenario())
