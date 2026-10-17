"""Compare consumed-fragment style invalidation on two normal release CLIs.

Run with uv run --no-project --with websockets==15.0.1 python <script> --help.
This measures shared style work, without screenshots or geometry reads. It is
not a complete layout benchmark or an engine-wide memory/CPU guarantee.
"""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import json
import os
import statistics
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from moli_benchmark.raw_cdp import connect_raw_cdp
from moli_benchmark.target_serve import start_target_serve, stop_target_serve

SETUP = r"""(() => {
  if(!document.documentElement)document.appendChild(document.createElement('html'));
  if(!document.head)document.documentElement.appendChild(document.createElement('head'));
  if(!document.body)document.documentElement.appendChild(document.createElement('body'));
  document.head.innerHTML='<style>.item{color:rgb(1,2,3)}</style>';
  document.body.innerHTML='<main id=list></main><aside id=other></aside>';
  window.items=[];
  for(let i=0;i<NODES;i++){
    const item=document.createElement('div');item.className='item';
    item.textContent='ABCD';list.appendChild(item);items.push(item);
  }
  window.iteration=0;
  window.expectedFirst='rgb(1, 2, 3)';
  window.readStyles=() => {
    let checked=0;
    for(const item of items){
      const color=getComputedStyle(item).color;
      const expected=checked===0?expectedFirst:'rgb(1, 2, 3)';
      if(color!==expected)throw Error(JSON.stringify({checked,color,expected}));
      checked++;
    }
    return checked;
  };
  return readStyles();
})()"""

OPERATIONS = {
    "append": """const item=document.createElement('div');
        item.className='item';item.textContent='ABCD';list.append(item);items.push(item);""",
    "reparent": """const item=items[0];
        (item.parentElement===list?other:list).append(item);""",
    "style": "items[0].style.color=expectedFirst=(iteration%2?'rgb(1, 2, 3)':'rgb(4, 5, 6)');",
}


def snapshot(pid: int) -> dict:
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    threads = {}
    for path in Path(f"/proc/{pid}/task").iterdir():
        try:
            stat = (path / "stat").read_text().rsplit(")", 1)[1].split()
            threads[f"{path.name}:{stat[19]}"] = int(
                (path / "schedstat").read_text().split()[0]
            )
        except FileNotFoundError:
            continue
    return {
        "monotonic_ns": time.monotonic_ns(),
        "identity": f"{pid}:{fields[19]}",
        "ticks": int(fields[11]) + int(fields[12]),
        "threads_schedstat_ns": threads,
    }


def pss(pid: int) -> int:
    return next(
        int(line.split()[1]) * 1024
        for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines()
        if line.startswith("Pss:")
    )


async def one_run(args, binary: Path, mode: str, scenario: str, record: dict):
    handle = client = None
    try:
        handle = start_target_serve(
            "moli", binary, 15, extra_args=("--layout",) if mode == "OnDemand" else ()
        )
        pid = handle.process.pid
        record.update(command=handle.command, pid=pid, root=snapshot(pid)["identity"])
        client = await connect_raw_cdp(handle.endpoint)

        async def command(method, params=None, session=None):
            ident = await client.send(method, params, session_id=session)
            reply, messages = await client.recv_until_id(ident, timeout=30)
            errors = [
                m for m in messages if m.get("method") == "Runtime.exceptionThrown"
            ]
            record.setdefault("runtime_errors", []).extend(errors)
            if errors:
                raise RuntimeError(errors)
            return reply["result"]

        target = (await command("Target.createTarget", {"url": "about:blank"}))[
            "targetId"
        ]
        session = (
            await command(
                "Target.attachToTarget", {"targetId": target, "flatten": True}
            )
        )["sessionId"]
        await command("Runtime.enable", {}, session)

        async def evaluate(source):
            result = await command(
                "Runtime.evaluate",
                {"expression": source, "returnByValue": True},
                session,
            )
            if "exceptionDetails" in result:
                raise RuntimeError(result)
            return result["result"].get("value")

        def pin():
            if args.cpu is not None:
                for path in Path(f"/proc/{pid}/task").iterdir():
                    try:
                        os.sched_setaffinity(int(path.name), {args.cpu})
                    except ProcessLookupError:
                        pass

        def affinities():
            return {
                path.name: sorted(os.sched_getaffinity(int(path.name)))
                for path in Path(f"/proc/{pid}/task").iterdir()
            }

        pin()
        assert await evaluate(SETUP.replace("NODES", str(args.nodes))) == args.nodes
        body = "(()=>{" + OPERATIONS[scenario] + "iteration++;return readStyles();})()"
        for _ in range(4):
            await evaluate(body)
        # Rebuild the same starting DOM after warmup, outside the measurement.
        assert await evaluate(SETUP.replace("NODES", str(args.nodes))) == args.nodes
        await command("HeapProfiler.collectGarbage", {}, session)
        pin()
        record["affinity_before"] = affinities()
        record["pss_before"] = pss(pid)
        before = snapshot(pid)
        started = time.perf_counter()
        results = []
        for index in range(args.iterations):
            value = await evaluate(body)
            expected = args.nodes + index + 1 if scenario == "append" else args.nodes
            assert value == expected, (value, expected)
            results.append(value)
        wall_ms = (time.perf_counter() - started) * 1000
        after = snapshot(pid)
        assert before["identity"] == after["identity"] == record["root"]
        old, new = before["threads_schedstat_ns"], after["threads_schedstat_ns"]
        assert old.keys() == new.keys(), "thread churn needs a separate CPU analysis"
        assert all(new[key] >= old[key] for key in old)
        record.update(
            wall_ms=wall_ms,
            cpu_ms=sum(new[key] - old[key] for key in old) / 1e6,
            cpu_scope="observed root threads at both boundaries; not descendant/lifetime CPU",
            process_cpu_ms=(after["ticks"] - before["ticks"])
            * 1000
            / os.sysconf("SC_CLK_TCK"),
            before=before,
            after=after,
            results=results,
        )
        await command("HeapProfiler.collectGarbage", {}, session)
        record["pss_after"] = pss(pid)
        record["affinity_after"] = affinities()
        if args.cpu is not None:
            for group in (record["affinity_before"], record["affinity_after"]):
                assert all(cpus == [args.cpu] for cpus in group.values())
        await command("Target.closeTarget", {"targetId": target})
        record["business_ok"] = True
    finally:
        try:
            if client is not None:
                await client.websocket.close()
        finally:
            if handle is not None:
                record["stop"] = stop_target_serve(handle)
                record["root_exited"] = (
                    handle.process.poll() is not None
                    and not Path(f"/proc/{handle.process.pid}").exists()
                )
                if not record["root_exited"]:
                    raise RuntimeError("benchmark process did not exit")


async def main(args):
    args.output.mkdir(parents=True, exist_ok=False)
    metadata = {
        "args": {
            key: str(value) if isinstance(value, Path) else value
            for key, value in vars(args).items()
        },
        "driver_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "binaries": {
            name: hashlib.sha256(path.read_bytes()).hexdigest()
            for name, path in (("before", args.before), ("after", args.after))
        },
        "removed_moli_environment_names": sorted(
            key for key in os.environ if key.startswith("MOLI_")
        ),
        "scope": "normal release CLIs, shared computed-style work; no layout reads or screenshots",
    }
    for key in metadata["removed_moli_environment_names"]:
        del os.environ[key]
    (args.output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    records = []
    for scenario in OPERATIONS:
        for mode in ("Mock", "OnDemand"):
            for repeat in range(args.repetitions):
                order = (("before", args.before), ("after", args.after))
                if repeat % 2:
                    order = tuple(reversed(order))
                for cohort, binary in order:
                    record = {
                        "scenario": scenario,
                        "mode": mode,
                        "repeat": repeat,
                        "cohort": cohort,
                    }
                    try:
                        await one_run(args, binary, mode, scenario, record)
                    except BaseException as error:
                        record["error"] = repr(error)
                        raise
                    finally:
                        path = args.output / f"{scenario}-{mode}-{repeat}-{cohort}.json"
                        path.write_text(json.dumps(record, indent=2) + "\n")
                        print(
                            json.dumps(
                                {
                                    key: record.get(key)
                                    for key in (
                                        "scenario",
                                        "mode",
                                        "repeat",
                                        "cohort",
                                        "wall_ms",
                                        "cpu_ms",
                                        "error",
                                    )
                                }
                            ),
                            flush=True,
                        )
                    records.append(record)
    summary = []
    for scenario in OPERATIONS:
        for mode in ("Mock", "OnDemand"):
            row = {"scenario": scenario, "mode": mode}
            for metric in ("wall_ms", "cpu_ms", "process_cpu_ms", "pss_after"):
                before, after = [
                    statistics.median(
                        record[metric]
                        for record in records
                        if record["scenario"] == scenario
                        and record["mode"] == mode
                        and record["cohort"] == cohort
                    )
                    for cohort in ("before", "after")
                ]
                row[metric] = {
                    "before": before,
                    "after": after,
                    "change_percent": (after / before - 1) * 100 if before else None,
                }
            summary.append(row)
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cpu", type=int, default=0)
    parser.add_argument("--nodes", type=int, default=1024)
    parser.add_argument("--iterations", type=int, default=64)
    parser.add_argument("--repetitions", type=int, default=6)
    args = parser.parse_args()
    for name in ("nodes", "iterations", "repetitions"):
        if getattr(args, name) < 1:
            parser.error(f"{name} must be positive")
    args.before = args.before.resolve(strict=True)
    args.after = args.after.resolve(strict=True)
    asyncio.run(main(args))
