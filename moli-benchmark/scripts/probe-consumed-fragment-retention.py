"""Attribute consumed-fragment memory costs on two frozen Linux release builds.

Run with uv run --no-project --with websockets==15.0.1 python <script> --help.
The node count stays fixed. Report raw smaps, V8 heap usage, process CPU,
post-close retention, and a per-process THP-disabled intervention separately.
This is a diagnostic probe, not a whole-browser memory or throughput guarantee.
"""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import json
import os
import re
import statistics
import sys
import tempfile
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from moli_benchmark.raw_cdp import connect_raw_cdp
from moli_benchmark.target_serve import start_target_serve, stop_target_serve

BOUNDARIES = r"""(async () => {
  if(!document.documentElement)document.appendChild(document.createElement('html'));
  if(!document.head)document.documentElement.appendChild(document.createElement('head'));
  if(!document.body)document.documentElement.appendChild(document.createElement('body'));
  const A='rgb(1, 2, 3)', B='rgb(9, 8, 7)';
  let checks=0;
  const equal=(actual,expected,label) => {
    if(actual!==expected)throw Error(JSON.stringify({label,actual,expected}));
    checks++;
  };
  document.body.innerHTML='';
  const style=document.createElement('style');
  style.textContent='.bucket{background-color:'+A+'}'+
    '.bucket:empty{background-color:'+B+'}'+
    '.item{color:'+A+';background-color:'+A+';border-top-color:'+A+
      ';border-right-color:'+A+';border-bottom-color:'+A+'}'+
    '.item:first-child{color:'+B+'}'+
    '.item:only-child{background-color:'+B+'}'+
    '.item:nth-child(2){border-top-color:'+B+'}'+
    '.marker ~ .item{border-right-color:'+B+'}'+
    '.bucket:has(> .marker) > .item{border-bottom-color:'+B+'}';
  document.head.appendChild(style);
  const bucket=document.createElement('div');
  bucket.className='bucket';document.body.appendChild(bucket);
  const fragment=document.createDocumentFragment();
  const items=Array.from({length:3},(_,index) => {
    const el=document.createElement('div');
    el.className='item'+(index===1?' marker':'');return el;
  });
  const check=() => {
    const live=Array.from(bucket.children);
    equal(getComputedStyle(bucket).backgroundColor,live.length?A:B,'empty');
    let preceded=false;
    const hasMarker=live.some(el=>el.classList.contains('marker'));
    live.forEach((el,index) => {
      const computed=getComputedStyle(el);
      for(const [property,expected] of [
        ['color',index===0?B:A],['backgroundColor',live.length===1?B:A],
        ['borderTopColor',index===1?B:A],['borderRightColor',preceded?B:A],
        ['borderBottomColor',hasMarker?B:A]
      ]){equal(computed[property],expected,property);}
      preceded=preceded||el.classList.contains('marker');
    });
  };
  check();
  for(let cycle=0;cycle<12;cycle++){
    // Reuse one fragment, with public style reads while it is nonempty.
    for(const item of items){fragment.appendChild(item);check();}
    bucket.appendChild(fragment);check();
    equal(fragment.childNodes.length,0,'consumed');
    // Change the real sibling topology again before the next style read.
    bucket.insertBefore(items[2],items[0]);
    bucket.removeChild(items[1]);
    fragment.appendChild(items[1]);
    bucket.insertBefore(fragment,bucket.firstChild);check();
  }
  const frame=document.createElement('iframe');
  document.body.appendChild(frame);
  await new Promise(resolve=>setTimeout(resolve,0));
  const other=frame.contentDocument;
  if(!other||!other.body)throw Error('missing active child document');
  const source=document.createElement('section');
  source.className='source';
  source.style.color='rgb(11, 22, 33)';document.body.appendChild(source);
  const sourceStyle=document.createElement('style');
  sourceStyle.textContent='.cross-doc{border-top-color:rgb(11,22,33)}'+
    '.source{background-color:'+A+'}'+
    '.source:has(> .cross-doc){background-color:'+B+'}';
  document.head.appendChild(sourceStyle);
  const childStyle=other.createElement('style');
  childStyle.textContent='.cross-doc{border-top-color:rgb(44,55,66)}'+
    'body{background-color:'+A+'}'+
    'body:has(> .cross-doc){background-color:'+B+'}';
  other.head.appendChild(childStyle);
  other.body.style.color='rgb(44, 55, 66)';
  const moving=document.createElement('span');
  moving.className='cross-doc';
  source.appendChild(moving);
  equal(getComputedStyle(moving).color,'rgb(11, 22, 33)','source inherited color');
  equal(getComputedStyle(moving).borderTopColor,'rgb(11, 22, 33)','source sheet');
  equal(getComputedStyle(source).backgroundColor,B,'source has child');
  equal(other.defaultView.getComputedStyle(other.body).backgroundColor,A,'child empty');
  for(let cycle=0;cycle<12;cycle++){
    fragment.appendChild(moving);
    // Drain pending work between removal and adoption into the child Document.
    equal(getComputedStyle(source).color,'rgb(11, 22, 33)','source mid-read');
    equal(getComputedStyle(source).backgroundColor,A,'source removal invalidation');
    other.body.appendChild(fragment);
    equal(moving.ownerDocument===other,true,'child owner');
    equal(other.defaultView.getComputedStyle(moving).color,
          'rgb(44, 55, 66)','child inherited color');
    equal(other.defaultView.getComputedStyle(moving).borderTopColor,
          'rgb(44, 55, 66)','child sheet');
    equal(other.defaultView.getComputedStyle(other.body).backgroundColor,
          B,'child insertion invalidation');
    fragment.appendChild(moving);
    source.appendChild(fragment);
    equal(moving.ownerDocument===document,true,'returned owner');
    equal(other.defaultView.getComputedStyle(other.body).backgroundColor,
          A,'child removal invalidation');
    equal(getComputedStyle(moving).color,'rgb(11, 22, 33)','returned color');
    equal(getComputedStyle(moving).borderTopColor,'rgb(11, 22, 33)','returned sheet');
    equal(getComputedStyle(source).backgroundColor,B,'source reinsertion invalidation');
  }
  frame.remove();source.remove();sourceStyle.remove();bucket.remove();style.remove();
  return {checks,fragmentReuseCycles:12,crossDocumentCycles:12};
})()"""

SETUP = r"""(() => {
  if(!document.documentElement)document.appendChild(document.createElement('html'));
  if(!document.head)document.documentElement.appendChild(document.createElement('head'));
  if(!document.body)document.documentElement.appendChild(document.createElement('body'));
  document.head.innerHTML='<style>.item{color:rgb(1,2,3)}</style>';
  document.body.innerHTML='<main id="list"></main>';
  window.items=[];
  for(let index=0;index<NODES;index++){
    const item=document.createElement('div');
    item.className='item';item.textContent='ABCD';
    list.appendChild(item);items.push(item);
  }
  window.fragment=document.createDocumentFragment();
  window.iteration=0;
  window.readStyles=() => {
    for(const item of items){
      const expected=item===items[0]&&SCENARIO==='style'&&iteration%2
        ?'rgb(4, 5, 6)':'rgb(1, 2, 3)';
      const actual=getComputedStyle(item).color;
      if(actual!==expected)throw Error(JSON.stringify({actual,expected}));
    }
    if(list.children.length!==NODES||items.length!==NODES)
      throw Error('fixed DOM cardinality changed');
    return items.length;
  };
  return readStyles();
})()"""

OPERATIONS = {
    "style": """
      iteration++;
      items[0].style.color=iteration%2?'rgb(4,5,6)':'rgb(1,2,3)';
      return readStyles();
    """,
    "churn": """
      const removed=items.shift();removed.remove();
      const item=document.createElement('div');
      item.className='item';item.textContent='ABCD';
      list.append(item);items.push(item);iteration++;
      return readStyles();
    """,
    "reuse": """
      const first=items.shift();fragment.appendChild(first);
      list.appendChild(fragment);items.push(first);iteration++;
      return readStyles();
    """,
}


def cpu_snapshot(pid: int) -> dict:
    stat = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    threads = {}
    for task in Path(f"/proc/{pid}/task").iterdir():
        try:
            task_stat = (task / "stat").read_text().rsplit(")", 1)[1].split()
            threads[f"{task.name}:{task_stat[19]}"] = int(
                (task / "schedstat").read_text().split()[0]
            )
        except FileNotFoundError:
            continue
    return {
        "identity": f"{pid}:{stat[19]}",
        "ticks": int(stat[11]) + int(stat[12]),
        "threads": threads,
        "monotonic_ns": time.monotonic_ns(),
    }


def smaps(pid: int, binary: Path) -> dict:
    started = time.monotonic_ns()
    result: dict = {"maps": [], "classes": {}, "rollup": {}}
    text = Path(f"/proc/{pid}/smaps").read_text()
    current = None
    for line in text.splitlines():
        if re.match(r"^[0-9a-f]+-[0-9a-f]+ ", line):
            fields = line.split(maxsplit=5)
            name = fields[5] if len(fields) == 6 else ""
            kind = (
                "binary"
                if name == str(binary)
                else "heap"
                if name == "[heap]"
                else "anonymous"
                if fields[4] == "0"
                else "other_file"
            )
            current = {"range": fields[0], "name": name, "class": kind}
            result["maps"].append(current)
        elif current is not None and line.endswith(" kB"):
            key, value = line.split(":", 1)
            current[key] = int(value.split()[0]) * 1024
    for mapping in result["maps"]:
        group = result["classes"].setdefault(mapping["class"], {})
        for key in (
            "Pss",
            "Rss",
            "Anonymous",
            "AnonHugePages",
            "Private_Dirty",
            "Private_Clean",
            "Shared_Clean",
            "Shared_Dirty",
            "SwapPss",
        ):
            group[key] = group.get(key, 0) + mapping.get(key, 0)
    for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines():
        if line.endswith(" kB"):
            key, value = line.split(":", 1)
            result["rollup"][key] = int(value.split()[0]) * 1024
    result["status"] = {
        line.split(":", 1)[0]: line.split(":", 1)[1].strip()
        for line in Path(f"/proc/{pid}/status").read_text().splitlines()
        if line.startswith(("THP_enabled:", "Threads:", "VmRSS:"))
    }
    result["capture_ns"] = time.monotonic_ns() - started
    return result


async def one_run(args, binary, mode, scenario, thp, record):
    handle = client = None
    with tempfile.TemporaryDirectory(prefix="moli-fragment-thp-") as temporary:
        executable = binary
        if thp == "disabled":
            executable = Path(temporary) / "moli"
            executable.write_text(
                "#!" + sys.executable + "\nimport ctypes,os,sys\n"
                "libc=ctypes.CDLL(None,use_errno=True)\n"
                "if libc.prctl(41,1,0,0,0)!=0:raise OSError(ctypes.get_errno())\n"
                f"os.execv({str(binary)!r},[{str(binary)!r},*sys.argv[1:]])\n"
            )
            executable.chmod(0o700)
        try:
            handle = start_target_serve(
                "moli",
                executable,
                15,
                extra_args=("--layout",) if mode == "OnDemand" else (),
            )
            pid = handle.process.pid
            assert Path(f"/proc/{pid}/exe").resolve() == binary
            identity = cpu_snapshot(pid)["identity"]
            record.update(pid=pid, identity=identity, command=handle.command)
            client = await connect_raw_cdp(handle.endpoint)

            async def command(method, params=None, session=None):
                ident = await client.send(method, params, session_id=session)
                reply, messages = await client.recv_until_id(ident, timeout=60)
                errors = [
                    message
                    for message in messages
                    if message.get("method") == "Runtime.exceptionThrown"
                ]
                if errors:
                    raise RuntimeError(errors)
                if "error" in reply:
                    raise RuntimeError(reply)
                return reply["result"]

            async def page():
                target = (await command("Target.createTarget", {"url": "about:blank"}))[
                    "targetId"
                ]
                session = (
                    await command(
                        "Target.attachToTarget",
                        {
                            "targetId": target,
                            "flatten": True,
                        },
                    )
                )["sessionId"]
                await command("Runtime.enable", {}, session)
                return target, session

            async def evaluate(source, session):
                result = await command(
                    "Runtime.evaluate",
                    {
                        "expression": source,
                        "returnByValue": True,
                        "awaitPromise": True,
                    },
                    session,
                )
                if "exceptionDetails" in result:
                    raise RuntimeError(result)
                return result["result"].get("value")

            def pin():
                for task in Path(f"/proc/{pid}/task").iterdir():
                    os.sched_setaffinity(int(task.name), {args.cpu})

            pin()
            record["server_before"] = smaps(pid, binary)
            if thp == "disabled":
                assert record["server_before"]["status"]["THP_enabled"] == "0"
            target, session = await page()
            pin()
            if args.boundaries:
                record["boundaries"] = await evaluate(BOUNDARIES, session)
            setup = SETUP.replace("NODES", str(args.nodes)).replace(
                "SCENARIO", json.dumps(scenario)
            )
            assert await evaluate(setup, session) == args.nodes
            body = "(() => {" + OPERATIONS[scenario] + "})()"
            if args.warmup:
                for _ in range(args.warmup):
                    assert await evaluate(body, session) == args.nodes
                assert await evaluate(setup, session) == args.nodes
            record["checkpoints"] = []
            completed = 0
            for count in args.checkpoints:
                before = cpu_snapshot(pid)
                for _ in range(count - completed):
                    assert await evaluate(body, session) == args.nodes
                after = cpu_snapshot(pid)
                assert before["identity"] == after["identity"] == identity
                assert before["threads"].keys() == after["threads"].keys()
                for _ in range(args.gc_passes):
                    await command("HeapProfiler.collectGarbage", {}, session)
                await asyncio.sleep(args.idle_ms / 1000)
                heap = await command("Runtime.getHeapUsage", {}, session)
                checkpoint = {
                    "iteration": count,
                    "cpu_ms": sum(
                        after["threads"][key] - value
                        for key, value in before["threads"].items()
                    )
                    / 1e6,
                    "process_tick_cpu_ms": (after["ticks"] - before["ticks"])
                    * 1000
                    / os.sysconf("SC_CLK_TCK"),
                    "elapsed_ms": (after["monotonic_ns"] - before["monotonic_ns"])
                    / 1e6,
                    "heap": heap,
                    "memory": smaps(pid, binary),
                    "affinities": [
                        sorted(os.sched_getaffinity(int(task.name)))
                        for task in Path(f"/proc/{pid}/task").iterdir()
                    ],
                }
                if args.diagnostics:
                    checkpoint["diagnostics"] = await command(
                        "HeapProfiler.moliDiagnostics", {}, session
                    )
                assert all(cpus == [args.cpu] for cpus in checkpoint["affinities"])
                record["checkpoints"].append(checkpoint)
                completed = count
            extra_pages = []
            for count in range(2, args.pages + 1):
                new_target, new_session = await page()
                pin()
                assert await evaluate(setup, new_session) == args.nodes
                extra_pages.append((new_target, new_session))
                if count & (count - 1) == 0 or count == args.pages:
                    for _, current_session in [(target, session), *extra_pages]:
                        for _ in range(args.gc_passes):
                            await command(
                                "HeapProfiler.collectGarbage", {}, current_session
                            )
                    await asyncio.sleep(args.idle_ms / 1000)
                    sample = {"pages": count, "memory": smaps(pid, binary)}
                    if args.diagnostics:
                        sample["diagnostics"] = await command(
                            "HeapProfiler.moliDiagnostics", {}, session
                        )
                    record.setdefault("multi_page", []).append(sample)
            for old_target, _ in reversed(extra_pages):
                await command("Target.closeTarget", {"targetId": old_target})
            assert (await command("Target.closeTarget", {"targetId": target}))[
                "success"
            ]
            for delay in (0.05, 0.25):
                await asyncio.sleep(delay)
                record.setdefault("closed", []).append(smaps(pid, binary))
            # A fresh page must not inherit references or styling from the old page.
            target, session = await page()
            record["fresh_heap"] = await command("Runtime.getHeapUsage", {}, session)
            record["fresh_diagnostics"] = await command(
                "HeapProfiler.moliDiagnostics", {}, session
            )
            assert (
                record["fresh_diagnostics"]["connection"]["pageNavigationEngineCount"]
                == 1
            )
            assert (
                record["fresh_diagnostics"]["isolateScope"][
                    "documentIsolateAccounting"
                ]["live"]
                == 1
            )
            assert (
                record["fresh_diagnostics"]["isolateScope"]["documentContextCount"] == 1
            )
            await command("Target.closeTarget", {"targetId": target})
            record["business_ok"] = True
        finally:
            try:
                if client is not None:
                    await client.websocket.close()
            finally:
                if handle is not None:
                    record["stop"] = stop_target_serve(handle)
                    record["root_exited"] = handle.process.poll() is not None
                    if not record["root_exited"]:
                        raise RuntimeError("target did not exit")


async def run_matrix(args):
    args.output.mkdir(parents=True, exist_ok=False)
    removed = sorted(key for key in os.environ if key.startswith("MOLI_"))
    for key in removed:
        del os.environ[key]
    metadata = {
        "args": {
            k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()
        },
        "driver_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "binaries": {
            key: {
                "path": str(path),
                "sha256": hashlib.file_digest(path.open("rb"), "sha256").hexdigest(),
            }
            for key, path in (("before", args.before), ("after", args.after))
        },
        "removed_environment_names": removed,
        "thp_policy": Path("/sys/kernel/mm/transparent_hugepage/enabled").read_text(),
        "started_unix_seconds": time.time(),
        "host": list(os.uname()),
        "cpu_scope": "observed root threads at batch boundaries, excluding GC and sampling",
        "memory_scope": "root smaps snapshots; V8 heap sampled separately; fixed DOM cardinality",
    }
    (args.output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    records = []
    for scenario in args.scenarios:
        for mode in args.modes:
            for thp in args.thp:
                for repeat in range(args.repetitions):
                    order = [("before", args.before), ("after", args.after)]
                    if repeat % 2:
                        order.reverse()
                    for cohort, binary in order:
                        record = {
                            "scenario": scenario,
                            "mode": mode,
                            "thp": thp,
                            "repeat": repeat,
                            "cohort": cohort,
                            "nodes": args.nodes,
                        }
                        try:
                            await one_run(args, binary, mode, scenario, thp, record)
                        except BaseException as error:
                            record["error"] = repr(error)
                            raise
                        finally:
                            name = f"{scenario}-{mode}-{thp}-{repeat}-{cohort}.json"
                            (args.output / name).write_text(
                                json.dumps(record, indent=2) + "\n"
                            )
                            print(
                                json.dumps(
                                    {
                                        k: record.get(k)
                                        for k in (
                                            "scenario",
                                            "mode",
                                            "thp",
                                            "repeat",
                                            "cohort",
                                            "business_ok",
                                            "error",
                                        )
                                    }
                                ),
                                flush=True,
                            )
                        records.append(record)
    summary = []
    for scenario in args.scenarios:
        for mode in args.modes:
            for thp in args.thp:
                row = {"scenario": scenario, "mode": mode, "thp": thp}
                for cohort in ("before", "after"):
                    group = [
                        record
                        for record in records
                        if record["scenario"] == scenario
                        and record["mode"] == mode
                        and record["thp"] == thp
                        and record["cohort"] == cohort
                    ]
                    row[cohort] = {
                        "pss_median": [
                            statistics.median(
                                r["checkpoints"][index]["memory"]["rollup"]["Pss"]
                                for r in group
                            )
                            for index in range(len(args.checkpoints))
                        ],
                        "used_heap_median": [
                            statistics.median(
                                r["checkpoints"][index]["heap"]["usedSize"]
                                for r in group
                            )
                            for index in range(len(args.checkpoints))
                        ],
                        "closed_pss_median": statistics.median(
                            r["closed"][-1]["rollup"]["Pss"] for r in group
                        ),
                    }
                summary.append(row)
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")


async def main(args):
    stopped = threading.Event()
    competitors = {}

    def watch_builds():
        while not stopped.is_set():
            for path in Path("/proc").iterdir():
                if not path.name.isdecimal():
                    continue
                try:
                    name = (path / "comm").read_text().strip()
                    if name in {"cargo", "rustc", "clippy-driver", "cargo-nextest"}:
                        competitors.setdefault(
                            path.name,
                            {
                                "command": name,
                                "first_seen_unix_seconds": time.time(),
                            },
                        )
                except (FileNotFoundError, ProcessLookupError):
                    continue
            stopped.wait(0.25)

    watcher = threading.Thread(target=watch_builds, daemon=True)
    watcher.start()
    try:
        await run_matrix(args)
    finally:
        stopped.set()
        watcher.join()
        if args.output.exists():
            (args.output / "host-competition.json").write_text(
                json.dumps(
                    {
                        "observed_builds": competitors,
                        "finished_unix_seconds": time.time(),
                        "poll_interval_seconds": 0.25,
                    },
                    indent=2,
                )
                + "\n"
            )
    if competitors:
        raise RuntimeError("build/test competition was observed; timing run is invalid")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cpu", type=int, default=0)
    parser.add_argument("--nodes", type=int, default=1024)
    parser.add_argument("--pages", type=int, default=1)
    parser.add_argument("--repetitions", type=int, default=6)
    parser.add_argument("--checkpoints", type=int, nargs="+", default=[0, 16, 64, 128])
    parser.add_argument(
        "--scenarios", choices=OPERATIONS, nargs="+", default=list(OPERATIONS)
    )
    parser.add_argument(
        "--modes", choices=["Mock", "OnDemand"], nargs="+", default=["Mock", "OnDemand"]
    )
    parser.add_argument(
        "--thp", choices=["default", "disabled"], nargs="+", default=["default"]
    )
    parser.add_argument("--idle-ms", type=int, default=250)
    parser.add_argument("--warmup", type=int, default=4)
    parser.add_argument("--gc-passes", type=int, default=2)
    parser.add_argument("--boundaries", action="store_true")
    parser.add_argument("--diagnostics", action="store_true")
    args = parser.parse_args()
    if min(args.nodes, args.pages, args.repetitions, args.gc_passes) < 1:
        parser.error("nodes/pages/repetitions/gc-passes must be positive")
    if min(args.idle_ms, args.warmup) < 0:
        parser.error("idle-ms/warmup must be nonnegative")
    if args.checkpoints != sorted(set(args.checkpoints)) or args.checkpoints[0] != 0:
        parser.error("checkpoints must start at zero and strictly increase")
    args.before = args.before.resolve(strict=True)
    args.after = args.after.resolve(strict=True)
    asyncio.run(main(args))
