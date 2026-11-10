from __future__ import annotations

import hashlib
import html
import json
import os
import platform
import statistics
from collections import defaultdict, deque
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import quote, urlparse


MIB = 1024 * 1024
STRESS_ASSETS = Path(__file__).with_name("stress_assets")
DEFAULT_D3 = STRESS_ASSETS / "d3.min.js"


def percentile(values: list[float], percent: float) -> float:
    ordered = sorted(float(value) for value in values if value is not None)
    position = (len(ordered) - 1) * percent / 100.0
    lower = int(position)
    upper = min(lower + 1, len(ordered) - 1)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def metric_stats(values: list[float]) -> dict[str, float | int | None]:
    if not values:
        return {
            "count": 0,
            "average": None,
            "p50": None,
            "p95": None,
            "p99": None,
            "max": None,
        }
    return {
        "count": len(values),
        "average": statistics.fmean(values),
        "p50": percentile(values, 50),
        "p95": percentile(values, 95),
        "p99": percentile(values, 99),
        "max": max(values),
    }


def mib(value: float | int | None) -> float | None:
    return None if value is None else float(value) / MIB


def memory_metric(metric: dict) -> dict:
    converted = dict(metric)
    for key in (
        "initial",
        "first",
        "final",
        "minimum",
        "peak",
        "first_window_average",
        "last_window_average",
        "first_to_last_window_delta",
        "warm_slope_per_navigation",
        "warm_slope_per_100_navigations",
    ):
        converted[key] = mib(metric.get(key))
    return converted


def rolling_average(values: list[float | None], window: int) -> list[float | None]:
    queue: deque[float | None] = deque()
    total = 0.0
    count = 0
    result: list[float | None] = []
    for value in values:
        queue.append(value)
        if value is not None:
            total += float(value)
            count += 1
        if len(queue) > window:
            removed = queue.popleft()
            if removed is not None:
                total -= float(removed)
                count -= 1
        result.append(total / count if count else None)
    return result


def regression(points: list[tuple[int, float]]) -> tuple[float, float]:
    if len(points) < 2:
        return 0.0, points[0][1] if points else 0.0
    mean_x = statistics.fmean(point[0] for point in points)
    mean_y = statistics.fmean(point[1] for point in points)
    denominator = sum((point[0] - mean_x) ** 2 for point in points)
    slope = sum((point[0] - mean_x) * (point[1] - mean_y) for point in points) / denominator
    return slope, mean_y - slope * mean_x


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def cpu_model() -> str:
    try:
        for line in Path("/proc/cpuinfo").read_text(encoding="utf-8").splitlines():
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or "unknown"


def memory_total_gib() -> float | None:
    try:
        for line in Path("/proc/meminfo").read_text(encoding="utf-8").splitlines():
            if line.startswith("MemTotal:"):
                return float(line.split()[1]) / 1024 / 1024
    except OSError:
        pass
    return None


def build_report(payload: dict, d3_source: str) -> tuple[dict, str]:
    results = payload.get("results")
    if not isinstance(results, list) or len(results) != 1:
        raise ValueError("stress reports require exactly one benchmark result")
    if not d3_source.startswith("// https://d3js.org v7.8.4"):
        raise ValueError("expected the vendored D3.js 7.8.4 asset")
    result = results[0]
    rows = result["rows"]
    if len(rows) < 4:
        raise ValueError("stress reports require at least four navigation rows")
    run = result["summary"]
    nav_resources = result.get("navigation_resources")
    if not isinstance(nav_resources, dict) or not nav_resources.get("samples"):
        raise ValueError(
            "result has no navigation resource samples; run with "
            "--navigation-resource-samples"
        )
    nav_summary = nav_resources["summary"]
    periodic_summary = result["process"]["resources"]
    periodic = periodic_summary.get("samples")
    if not isinstance(periodic, list) or not periodic:
        raise ValueError(
            "result has no periodic resource samples; run with "
            "--periodic-resource-samples or use moli-stress run"
        )
    duration = (
        datetime.fromisoformat(result["finished_at"])
        - datetime.fromisoformat(result["started_at"])
    ).total_seconds()

    latency = {
        key: metric_stats([float(row[key]) for row in rows])
        for key in ("response_ms", "dcl_ms", "load_ms", "elapsed_ms")
    }
    grouped: dict[str, list[dict]] = defaultdict(list)
    for row in rows:
        grouped[urlparse(row["url"]).netloc].append(row)
    sites = []
    for site, site_rows in grouped.items():
        sites.append(
            {
                "site": site,
                "count": len(site_rows),
                "passes": sum(bool(row["ok"]) for row in site_rows),
                "response_ms": metric_stats([float(row["response_ms"]) for row in site_rows]),
                "dcl_ms": metric_stats([float(row["dcl_ms"]) for row in site_rows]),
                "load_ms": metric_stats([float(row["load_ms"]) for row in site_rows]),
            }
        )

    cpu = [
        float(sample["cpu_percent"])
        for sample in periodic
        if sample.get("cpu_percent") is not None
    ]
    cpu_stats = metric_stats(cpu)
    quarters = nav_summary["quarters"]
    quarter_delta = quarters[-1]["rss_bytes"]["average"] - quarters[0]["rss_bytes"]["average"]
    quarter_delta_percent = (
        quarters[-1]["rss_bytes"]["average"] / quarters[0]["rss_bytes"]["average"] - 1
    ) * 100
    binary = Path(result["binary"])
    metadata = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "commit": payload["repository"]["commit"],
        "repository_dirty": payload["repository"]["dirty"],
        "binary": str(binary),
        "binary_sha256": file_sha256(binary) if binary.is_file() else None,
        "binary_size_bytes": binary.stat().st_size if binary.is_file() else None,
        "cpu": cpu_model(),
        "logical_cpus": os.cpu_count(),
        "memory_total_gib": memory_total_gib(),
        "platform": platform.platform(),
        "chart_library": "D3.js 7.8.4",
        "chart_library_source": "moli_benchmark/stress_assets/d3.min.js",
    }
    sampling = {key: value for key, value in periodic_summary.items() if key != "samples"}
    summary = {
        "status": "pass" if run["failures"] == 0 and run["network_order_violations"] == 0 else "fail",
        "benchmark_exit_code": 0 if run["failures"] == 0 and run["network_order_violations"] == 0 else 1,
        "started_at": result["started_at"],
        "finished_at": result["finished_at"],
        "duration_seconds": duration,
        "throughput_navigations_per_second": run["attempted"] / duration,
        "rounds": payload.get("rounds"),
        "network_diagnostics": bool(payload.get("network_diagnostics")),
        "urls_per_round": (
            len(payload.get("urls", [])) // int(payload["rounds"])
            if payload.get("rounds")
            else len(grouped)
        ),
        "ready_ms": result["ready_ms"],
        "navigation": run,
        "latency": latency,
        "sites": sites,
        "memory": {
            "rss_mib": memory_metric(nav_summary["rss_bytes"]),
            "pss_mib": memory_metric(nav_summary["pss_bytes"]),
            "quarter_1_to_4_rss_delta_mib": quarter_delta / MIB,
            "quarter_1_to_4_rss_delta_percent": quarter_delta_percent,
            "quarters": [
                {
                    "quarter": quarter["quarter"],
                    "start_index": quarter["start_index"],
                    "end_index": quarter["end_index"],
                    "rss_average_mib": mib(quarter["rss_bytes"]["average"]),
                    "rss_peak_mib": mib(quarter["rss_bytes"]["peak"]),
                    "pss_average_mib": mib(quarter["pss_bytes"]["average"]),
                }
                for quarter in quarters
            ],
        },
        "cpu": cpu_stats,
        "sampling": sampling,
        "metadata": metadata,
        "process_returncode": result["process"]["returncode"],
    }

    raw_cpu = [sample.get("cpu_percent") for sample in periodic]
    smooth_cpu = rolling_average(raw_cpu, 20)
    resource_points = [
        {
            "x": round(float(sample["elapsed_ms"]) / 60000, 6),
            "rss": mib(sample.get("rss_bytes")),
            "pss": mib(sample.get("pss_bytes")),
            "cpu": sample.get("cpu_percent"),
            "cpu2": smooth,
        }
        for sample, smooth in zip(periodic, smooth_cpu)
    ]

    warm_start = len(rows) // 10
    warm_points = [
        (int(sample["index"]), mib(sample["rss_bytes"]))
        for sample in nav_resources["samples"]
        if int(sample["index"]) > warm_start and sample.get("rss_bytes") is not None
    ]
    warm_slope, warm_intercept = regression(warm_points)
    navigation_memory = [
        {
            "x": int(sample["index"]),
            "rss": mib(sample.get("rss_bytes")),
            "pss": mib(sample.get("pss_bytes")),
            "trend": (
                warm_slope * int(sample["index"]) + warm_intercept
                if int(sample["index"]) > warm_start
                else None
            ),
        }
        for sample in nav_resources["samples"]
    ]
    latency_points = [
        {
            "x": int(row["index"]),
            "response": float(row["response_ms"]),
            "dcl": float(row["dcl_ms"]),
            "load": float(row["load_ms"]),
            "context": urlparse(row["url"]).netloc,
        }
        for row in rows
    ]
    chart_data = {
        "resources": resource_points,
        "memory": navigation_memory,
        "latency": latency_points,
        "site_bars": [
            {
                "name": site["site"],
                "p50": site["load_ms"]["p50"],
                "p95": site["load_ms"]["p95"],
                "p99": site["load_ms"]["p99"],
            }
            for site in sites
        ],
        "quarter_bars": [
            {
                "name": f"Q{quarter['quarter']} #{quarter['start_index']}–{quarter['end_index']}",
                "average": mib(quarter["rss_bytes"]["average"]),
                "peak": mib(quarter["rss_bytes"]["peak"]),
            }
            for quarter in quarters
        ],
    }
    slowest = [
        {
            "index": row["index"],
            "site": urlparse(row["url"]).netloc,
            "response_ms": row["response_ms"],
            "dcl_ms": row["dcl_ms"],
            "load_ms": row["load_ms"],
            "elapsed_ms": row["elapsed_ms"],
        }
        for row in sorted(rows, key=lambda row: row["load_ms"], reverse=True)[:10]
    ]
    report_data = {"summary": summary, "charts": chart_data, "slowest": slowest}
    data_json = json.dumps(report_data, ensure_ascii=False, separators=(",", ":")).replace("</", "<\\/")
    html_report = HTML_TEMPLATE.replace("__D3_SOURCE__", d3_source).replace("__REPORT_DATA__", data_json)
    return summary, html_report


HTML_TEMPLATE = r'''<!doctype html>
<html lang="zh-CN"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Moli 顺序导航压力测试报告</title>
<style>
:root{color-scheme:dark;--bg:#07111f;--panel:#0f1d31;--line:#24344d;--text:#e2e8f0;--muted:#94a3b8;--green:#34d399;--amber:#fbbf24}
*{box-sizing:border-box}body{margin:0;background:radial-gradient(circle at 75% 0,#142948 0,transparent 34%),var(--bg);color:var(--text);font:15px/1.58 ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif}
main{max-width:1480px;margin:auto;padding:42px 26px 72px}h1{margin:0 0 8px;font-size:34px;letter-spacing:-.03em}h2{margin:0 0 10px;font-size:21px}p{margin:7px 0}code{color:#bfdbfe;overflow-wrap:anywhere}a{color:#7dd3fc}.muted{color:var(--muted)}
.header{display:flex;justify-content:space-between;gap:24px;align-items:flex-start;margin-bottom:26px}.badge{white-space:nowrap;padding:8px 13px;border-radius:999px;font-weight:750;letter-spacing:.04em}.badge.pass{color:#6ee7b7;background:#064e3b66;border:1px solid #10b98188}.badge.fail{color:#fda4af;background:#88133766;border:1px solid #fb718588}
.cards{display:grid;grid-template-columns:repeat(6,minmax(150px,1fr));gap:12px;margin:20px 0 26px}.card,.panel{background:linear-gradient(155deg,#13243aee,#0c182aee);border:1px solid var(--line);border-radius:14px;box-shadow:0 10px 32px #02061733}.card{padding:15px 16px}.card .label{color:var(--muted);font-size:12px;text-transform:uppercase;letter-spacing:.08em}.card .value{margin-top:5px;font-size:24px;font-weight:760}.card .sub{color:var(--muted);font-size:12px}
.panel{padding:20px;margin:14px 0}.callout{border-left:3px solid var(--amber);padding-left:16px}.grid-2{display:grid;grid-template-columns:1fr 1fr;gap:14px}.chart{position:relative;min-height:430px}.chart svg{width:100%;height:auto;display:block}.chart-controls{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin:4px 0 6px}.chart-controls button{border:1px solid #334766;background:#111f34;color:#cbd5e1;border-radius:7px;padding:5px 9px;cursor:pointer}.chart-controls button:hover{border-color:#60a5fa}.legend-item.off{opacity:.35;text-decoration:line-through}.tooltip{position:absolute;display:none;pointer-events:none;z-index:4;background:#020617ed;border:1px solid #475569;border-radius:8px;padding:8px 10px;color:#e2e8f0;font-size:12px;box-shadow:0 8px 25px #0008;min-width:145px}
table{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums}th,td{padding:10px 11px;text-align:right;border-bottom:1px solid #26364d}th{color:#a5b4fc;font-size:12px;white-space:nowrap}td:first-child,th:first-child{text-align:left}tr:last-child td{border-bottom:0}ul{margin:8px 0;padding-left:22px}.facts{display:grid;grid-template-columns:190px 1fr;gap:7px 14px}.facts dt{color:var(--muted)}.facts dd{margin:0;overflow-wrap:anywhere}footer{color:var(--muted);margin-top:24px;font-size:13px}
@media(max-width:1100px){.cards{grid-template-columns:repeat(3,1fr)}.grid-2{grid-template-columns:1fr}}@media(max-width:650px){main{padding:24px 12px}.header{display:block}.badge{display:inline-block;margin-top:14px}.cards{grid-template-columns:repeat(2,1fr)}.facts{grid-template-columns:1fr}.panel{padding:13px;overflow-x:auto}}
</style></head><body><main>
<div class="header"><div><h1 id="report-title">Moli 顺序导航压力测试报告</h1><p class="muted" id="run-description"></p><p class="muted" id="run-time"></p></div><span id="status" class="badge"></span></div>
<div class="cards">
 <div class="card"><div class="label">导航结果</div><div class="value" id="card-nav"></div><div class="sub" id="card-nav-sub"></div></div>
 <div class="card"><div class="label">总时长</div><div class="value" id="card-duration"></div><div class="sub" id="card-throughput"></div></div>
 <div class="card"><div class="label">Load P50 / P95</div><div class="value" id="card-load"></div><div class="sub" id="card-load-sub"></div></div>
 <div class="card"><div class="label">最终 / 峰值 RSS</div><div class="value" id="card-rss"></div><div class="sub">MiB</div></div>
 <div class="card"><div class="label">RSS 热身后斜率</div><div class="value" id="card-slope"></div><div class="sub">MiB / 100 navigations</div></div>
 <div class="card"><div class="label">CPU 平均 / 峰值</div><div class="value" id="card-cpu"></div><div class="sub" id="card-cpu-sub"></div></div>
</div>
<section class="panel"><h2>结论</h2><div class="callout" id="conclusion"></div></section>
<section class="panel"><h2>进程树 RSS / PSS / CPU 时间序列</h2><p class="muted">滚轮缩放，拖拽平移，双击或“重置”恢复；图例可切换序列；可导出当前图为 SVG。</p><div id="resource-chart" class="chart"></div></section>
<section class="panel"><h2>每次导航完成后的内存检查点</h2><div id="memory-chart" class="chart"></div></section>
<section class="panel"><h2>导航延迟曲线</h2><div id="latency-chart" class="chart"></div></section>
<div class="grid-2"><section class="panel"><h2>各站点 Load 延迟分位数</h2><div id="site-chart" class="chart"></div></section><section class="panel"><h2>四分段 RSS（等分导航序列）</h2><div id="quarter-chart" class="chart"></div></section></div>
<section class="panel"><h2>按站点延迟（ms）</h2><table><thead><tr><th>站点</th><th>通过</th><th>Response P50</th><th>Response P95</th><th>Load P50</th><th>Load P95</th><th>Load P99</th><th>Load Max</th></tr></thead><tbody id="site-table"></tbody></table></section>
<section class="panel"><h2>最慢的 10 次导航（ms）</h2><table><thead><tr><th>序号</th><th>站点</th><th>Response</th><th>DCL</th><th>Load</th><th>总耗时</th></tr></thead><tbody id="slow-table"></tbody></table></section>
<div class="grid-2"><section class="panel"><h2>采样质量</h2><dl class="facts" id="sampling-facts"></dl></section><section class="panel"><h2>运行环境</h2><dl class="facts" id="environment-facts"></dl></section></div>
<section class="panel"><h2>方法与限制</h2><ul>
 <li id="run-shape"></li>
 <li>二进制由 <code>cargo build --release --locked -p moli</code> 构建；参数沿用 sequential-navigation soak 的超时、Network diagnostics 与导航边界资源采样。</li>
 <li>RSS/PSS 是每次导航完成后的进程树检查点；CPU/RSS/PSS 时间序列由 procfs 每 100 ms 采样。CPU 的 100% 表示一个逻辑核。</li>
 <li>公开网站与公网链路不可控，因此延迟包含远端与网络波动。本报告没有 base/Chromium 对照，不用于声称相对性能提升。</li>
 <li>服务进程在采样完成后由 harness 主动 SIGTERM，原始 <code>process.returncode=143</code> 是正常收尾；benchmark 自身退出码为 0。</li>
</ul></section>
<footer id="footer"></footer>
</main>
<script>__D3_SOURCE__</script>
<script type="application/json" id="report-data">__REPORT_DATA__</script>
<script>
const R=JSON.parse(document.getElementById('report-data').textContent),S=R.summary,C=R.charts;
const colors={blue:'#60a5fa',green:'#34d399',amber:'#fbbf24',red:'#fb7185',purple:'#c084fc',cyan:'#22d3ee',muted:'#94a3b8'};
const fmt=(v,n=1)=>v==null?'—':Number(v).toLocaleString('zh-CN',{minimumFractionDigits:n,maximumFractionDigits:n});
const set=(id,value)=>document.getElementById(id).textContent=value;
set('report-title',`Moli ${S.navigation.attempted} 次顺序导航性能报告`);document.title=`Moli ${S.navigation.attempted} 次顺序导航性能报告`;set('run-description',`Commit ${S.metadata.commit.slice(0,12)} · ${S.urls_per_round} 个 URL × ${S.rounds} 个循环 · release 构建 · Network diagnostics ${S.network_diagnostics?'开启':'关闭'}`);set('run-time',`${S.started_at} — ${S.finished_at}`);set('run-shape',`本次运行 ${S.navigation.attempted} 次导航：${S.urls_per_round} 个 URL × ${S.rounds} 个循环；URL 顺序见原始 result.json。`); const badge=document.getElementById('status'); badge.classList.add(S.status); badge.textContent=`${S.status.toUpperCase()} · ${S.navigation.observable_passes} / ${S.navigation.attempted}`;
set('card-nav',`${S.navigation.observable_passes} / ${S.navigation.attempted}`); set('card-nav-sub',`失败 ${S.navigation.failures} · 恢复 ${S.navigation.recovery_attempts} · superseded ${S.navigation.superseded_passes}`);
set('card-duration',`${fmt(S.duration_seconds/60,2)} min`);set('card-throughput',`${fmt(S.throughput_navigations_per_second,3)} nav/s`);set('card-load',`${fmt(S.latency.load_ms.p50/1000,2)} / ${fmt(S.latency.load_ms.p95/1000,2)}s`);set('card-load-sub',`P99 ${fmt(S.latency.load_ms.p99/1000,2)}s`);
set('card-rss',`${fmt(S.memory.rss_mib.final)} / ${fmt(S.memory.rss_mib.peak)}`);set('card-slope',`${S.memory.rss_mib.warm_slope_per_100_navigations>=0?'+':''}${fmt(S.memory.rss_mib.warm_slope_per_100_navigations,2)}`);set('card-cpu',`${fmt(S.cpu.average)}% / ${fmt(S.cpu.max,0)}%`);set('card-cpu-sub',`P95 ${fmt(S.cpu.p95)}% · 100%=1 核`);
const tailSite=S.sites.reduce((best,site)=>site.load_ms.p95>best.load_ms.p95?site:best,S.sites[0]);document.getElementById('conclusion').innerHTML=`<p><strong>功能稳定性：</strong>${S.navigation.attempted} 次导航中 ${S.navigation.observable_passes} 次可观测成功；${S.navigation.failures} 次失败、${S.navigation.recovery_attempts} 次恢复、${S.navigation.network_order_violations} 个 Network 事件顺序违规。测试进程退出码为 ${S.benchmark_exit_code}。</p><p><strong>内存：</strong>热身后 RSS 线性斜率为 <strong>${S.memory.rss_mib.warm_slope_per_100_navigations>=0?'+':''}${fmt(S.memory.rss_mib.warm_slope_per_100_navigations,2)} MiB / 100 次</strong>；Q4 平均值比 Q1 高 <strong>${fmt(S.memory.quarter_1_to_4_rss_delta_mib,2)} MiB（${fmt(S.memory.quarter_1_to_4_rss_delta_percent,2)}%）</strong>。最终 RSS ${fmt(S.memory.rss_mib.final)} MiB，较 ${fmt(S.memory.rss_mib.peak)} MiB 峰值已回落。单次压力测试不足以单独判定泄漏；建议后续与固定网络/页面的基线重复跑对比。</p><p><strong>CPU：</strong>进程树平均 ${fmt(S.cpu.average)}%，P95 ${fmt(S.cpu.p95)}%，峰值 ${fmt(S.cpu.max)}%。该指标按逻辑核聚合，所以可超过 100%。</p><p><strong>延迟：</strong>全体 Load P50 ${fmt(S.latency.load_ms.p50)} ms、P95 ${fmt(S.latency.load_ms.p95)} ms、P99 ${fmt(S.latency.load_ms.p99)} ms；最大 P95 长尾来自 ${tailSite.site}（${fmt(tailSite.load_ms.p95)} ms）。</p>`;

function downloadSvg(svg,name){const clone=svg.cloneNode(true);clone.setAttribute('xmlns','http://www.w3.org/2000/svg');const blob=new Blob([new XMLSerializer().serializeToString(clone)],{type:'image/svg+xml'}),a=document.createElement('a');a.href=URL.createObjectURL(blob);a.download=name;a.click();setTimeout(()=>URL.revokeObjectURL(a.href),1000)}
let chartCounter=0;
function lineChart(selector,data,series,options){
 const root=d3.select(selector),W=1200,H=470,M={top:20,right:75,bottom:52,left:68},PW=W-M.left-M.right,PH=H-M.top-M.bottom,id=`clip-${++chartCounter}`,active=new Set(series.map(s=>s.key));
 const controls=root.append('div').attr('class','chart-controls');const legend=controls.append('span');const reset=controls.append('button').text('重置');const save=controls.append('button').text('导出 SVG');
 const tooltip=root.append('div').attr('class','tooltip'),svg=root.append('svg').attr('viewBox',`0 0 ${W} ${H}`).attr('role','img'),g=svg.append('g').attr('transform',`translate(${M.left},${M.top})`);
 const x0=d3.scaleLinear().domain(d3.extent(data,d=>d.x)).range([0,PW]);let currentX=x0;
 const domain=axis=>{const values=[];for(const d of data)for(const s of series)if(s.axis===axis&&d[s.key]!=null)values.push(d[s.key]);let lo=options.zeroY?0:d3.min(values),hi=d3.max(values);if(lo===hi)hi=lo+1;return [options.zeroY?0:lo-(hi-lo)*.06,hi+(hi-lo)*.08]};
 const yL=d3.scaleLinear().domain(domain('left')).nice().range([PH,0]),hasRight=series.some(s=>s.axis==='right'),yR=d3.scaleLinear().domain(hasRight?domain('right'):[0,1]).nice().range([PH,0]);
 g.append('g').attr('class','grid').call(d3.axisLeft(yL).ticks(7).tickSize(-PW).tickFormat('')).call(x=>x.select('.domain').remove()).call(x=>x.selectAll('line').attr('stroke','#334155').attr('opacity',.38));
 const xAxis=g.append('g').attr('transform',`translate(0,${PH})`),leftAxis=g.append('g').call(d3.axisLeft(yL).ticks(7));if(hasRight)g.append('g').attr('transform',`translate(${PW},0)`).call(d3.axisRight(yR).ticks(7));
 svg.append('text').attr('x',M.left+PW/2).attr('y',H-10).attr('text-anchor','middle').attr('fill','#94a3b8').text(options.xLabel);svg.append('text').attr('transform','rotate(-90)').attr('x',-(M.top+PH/2)).attr('y',16).attr('text-anchor','middle').attr('fill','#94a3b8').text(options.leftLabel);if(hasRight)svg.append('text').attr('transform','rotate(90)').attr('x',M.top+PH/2).attr('y',-W+16).attr('text-anchor','middle').attr('fill','#94a3b8').text(options.rightLabel);
 g.append('defs').append('clipPath').attr('id',id).append('rect').attr('width',PW).attr('height',PH);const lines=g.append('g').attr('clip-path',`url(#${id})`),paths=new Map();
 const lineFor=(s,x)=>d3.line().defined(d=>d[s.key]!=null).x(d=>x(d.x)).y(d=>(s.axis==='right'?yR:yL)(d[s.key]))(data);
 for(const s of series){const path=lines.append('path').datum(data).attr('fill','none').attr('stroke',s.color).attr('stroke-width',s.width||1.5).attr('stroke-opacity',s.opacity??1).attr('vector-effect','non-scaling-stroke').attr('d',lineFor(s,x0));paths.set(s.key,path);const b=legend.append('button').attr('class','legend-item').style('color',s.color).text(`● ${s.label}`).on('click',function(){if(active.has(s.key)){active.delete(s.key);d3.select(this).classed('off',true);path.style('display','none')}else{active.add(s.key);d3.select(this).classed('off',false);path.style('display',null)}})}
 const cross=g.append('line').attr('y1',0).attr('y2',PH).attr('stroke','#e2e8f0').attr('stroke-dasharray','3,3').style('display','none');
 function redraw(x){currentX=x;xAxis.call(d3.axisBottom(x).ticks(10));for(const s of series)paths.get(s.key).attr('d',lineFor(s,x))}redraw(x0);
 const zoom=d3.zoom().scaleExtent([1,80]).extent([[0,0],[PW,PH]]).translateExtent([[0,0],[PW,PH]]).on('zoom',e=>redraw(e.transform.rescaleX(x0)));svg.call(zoom).on('dblclick.zoom',()=>svg.transition().duration(250).call(zoom.transform,d3.zoomIdentity));reset.on('click',()=>svg.transition().duration(250).call(zoom.transform,d3.zoomIdentity));save.on('click',()=>downloadSvg(svg.node(),options.filename));
 svg.on('mousemove.hover',event=>{const [sx,sy]=d3.pointer(event,svg.node()),mx=sx-M.left,my=sy-M.top;if(mx<0||mx>PW||my<0||my>PH){cross.style('display','none');tooltip.style('display','none');return}const xv=currentX.invert(mx),i=d3.bisector(d=>d.x).center(data,xv),d=data[Math.max(0,Math.min(data.length-1,i))];cross.attr('x1',currentX(d.x)).attr('x2',currentX(d.x)).style('display',null);let body=`<strong>${options.xValue(d.x)}</strong>${d.context?`<br>${d.context}`:''}`;for(const s of series)if(active.has(s.key)&&d[s.key]!=null)body+=`<br><span style="color:${s.color}">●</span> ${s.label}: ${fmt(d[s.key],s.digits??1)}${s.unit||''}`;tooltip.html(body).style('display','block').style('left',`${Math.min(82,Math.max(2,sx/W*100))}%`).style('top',`${Math.max(9,sy/H*100-4)}%`)}).on('mouseleave.hover',()=>{cross.style('display','none');tooltip.style('display','none')});
}
function groupedBars(selector,data,keys,options){
 const root=d3.select(selector),W=720,H=430,M={top:25,right:18,bottom:85,left:65},PW=W-M.left-M.right,PH=H-M.top-M.bottom,tooltip=root.append('div').attr('class','tooltip'),svg=root.append('svg').attr('viewBox',`0 0 ${W} ${H}`),g=svg.append('g').attr('transform',`translate(${M.left},${M.top})`),x=d3.scaleBand().domain(data.map(d=>d.name)).range([0,PW]).padding(.18),x1=d3.scaleBand().domain(keys.map(k=>k.key)).range([0,x.bandwidth()]).padding(.08),y=d3.scaleLinear().domain([0,d3.max(data,d=>d3.max(keys,k=>d[k.key]))*1.08]).nice().range([PH,0]);
 g.append('g').call(d3.axisLeft(y).ticks(7));g.append('g').attr('transform',`translate(0,${PH})`).call(d3.axisBottom(x)).selectAll('text').attr('transform','rotate(-20)').style('text-anchor','end');g.append('g').call(d3.axisLeft(y).ticks(7).tickSize(-PW).tickFormat('')).call(z=>z.select('.domain').remove()).call(z=>z.selectAll('line').attr('stroke','#334155').attr('opacity',.38));
 const groups=g.selectAll('.bar-group').data(data).join('g').attr('transform',d=>`translate(${x(d.name)},0)`);groups.selectAll('rect').data(d=>keys.map(k=>({name:d.name,key:k,value:d[k.key]}))).join('rect').attr('x',d=>x1(d.key.key)).attr('y',d=>y(d.value)).attr('width',x1.bandwidth()).attr('height',d=>PH-y(d.value)).attr('rx',2).attr('fill',d=>d.key.color).on('mousemove',(event,d)=>{const [px,py]=d3.pointer(event,root.node());tooltip.html(`<strong>${d.name}</strong><br><span style="color:${d.key.color}">●</span> ${d.key.label}: ${fmt(d.value)} ${options.unit}`).style('display','block').style('left',`${Math.min(78,px/root.node().clientWidth*100)}%`).style('top',`${Math.max(5,py-40)}px`)}).on('mouseleave',()=>tooltip.style('display','none'));
 const legend=root.insert('div',':first-child').attr('class','chart-controls');for(const k of keys)legend.append('span').style('color',k.color).style('margin-right','12px').text(`● ${k.label}`);svg.append('text').attr('transform','rotate(-90)').attr('x',-(M.top+PH/2)).attr('y',16).attr('text-anchor','middle').attr('fill','#94a3b8').text(options.yLabel);
}
lineChart('#resource-chart',C.resources,[{key:'rss',label:'RSS',axis:'left',color:colors.red,unit:' MiB'},{key:'pss',label:'PSS',axis:'left',color:colors.purple,unit:' MiB'},{key:'cpu',label:'CPU 原始值（100 ms）',axis:'right',color:colors.amber,unit:'%',width:.7,opacity:.22},{key:'cpu2',label:'CPU 2 秒移动平均',axis:'right',color:'#fde047',unit:'%',width:2}],{xLabel:'运行时间（分钟）',leftLabel:'内存（MiB）',rightLabel:'CPU（%，100%=一个逻辑核）',xValue:x=>`${fmt(x,2)} 分钟`,filename:'moli-rss-pss-cpu.svg'});
lineChart('#memory-chart',C.memory,[{key:'rss',label:'RSS',axis:'left',color:colors.red,unit:' MiB'},{key:'pss',label:'PSS',axis:'left',color:colors.purple,unit:' MiB'},{key:'trend',label:'RSS 热身后线性趋势',axis:'left',color:colors.amber,unit:' MiB',width:2}],{xLabel:'导航序号',leftLabel:'内存（MiB）',xValue:x=>`导航 #${Math.round(x)}`,filename:'moli-navigation-memory.svg'});
lineChart('#latency-chart',C.latency,[{key:'response',label:'Page.navigate 响应',axis:'left',color:colors.blue,unit:' ms'},{key:'dcl',label:'DOMContentLoaded',axis:'left',color:colors.green,unit:' ms'},{key:'load',label:'Load',axis:'left',color:colors.amber,unit:' ms'}],{xLabel:'导航序号',leftLabel:'耗时（ms）',zeroY:true,xValue:x=>`导航 #${Math.round(x)}`,filename:'moli-navigation-latency.svg'});
groupedBars('#site-chart',C.site_bars,[{key:'p50',label:'P50',color:colors.blue},{key:'p95',label:'P95',color:colors.amber},{key:'p99',label:'P99',color:colors.red}],{unit:'ms',yLabel:'Load（ms）'});groupedBars('#quarter-chart',C.quarter_bars,[{key:'average',label:'平均 RSS',color:colors.blue},{key:'peak',label:'峰值 RSS',color:colors.red}],{unit:'MiB',yLabel:'RSS（MiB）'});
function row(table,values){const tr=document.createElement('tr');for(const value of values){const td=document.createElement('td');td.textContent=value;tr.appendChild(td)}document.getElementById(table).appendChild(tr)}
for(const site of S.sites)row('site-table',[site.site,`${site.passes} / ${site.count}`,fmt(site.response_ms.p50),fmt(site.response_ms.p95),fmt(site.load_ms.p50),fmt(site.load_ms.p95),fmt(site.load_ms.p99),fmt(site.load_ms.max)]);for(const item of R.slowest)row('slow-table',[`#${item.index}`,item.site,fmt(item.response_ms),fmt(item.dcl_ms),fmt(item.load_ms),fmt(item.elapsed_ms)]);
function facts(id,items){const dl=document.getElementById(id);for(const [term,value] of items){const dt=document.createElement('dt'),dd=document.createElement('dd');dt.textContent=term;dd.textContent=value;dl.append(dt,dd)}}
facts('sampling-facts',[['方法',S.sampling.sampling_method],['周期样本',`${S.sampling.sample_count.toLocaleString()}（CPU 有效 ${S.cpu.count.toLocaleString()}）`],['目标 / 实际平均间隔',`${fmt(S.sampling.interval_seconds*1000,0)} / ${fmt(S.sampling.observed_interval_ms.average,2)} ms`],['迟到样本',`${S.sampling.late_sample_count} / ${S.sampling.sample_count}（最大间隔 ${fmt(S.sampling.observed_interval_ms.max)} ms）`],['PSS 完整样本',`${S.sampling.pss_complete_samples} / ${S.sampling.sample_count}`],['采样耗时平均 / 最大',`${fmt(S.sampling.capture_duration_ms.average,2)} / ${fmt(S.sampling.capture_duration_ms.max,2)} ms`],['观察器错误',String(S.sampling.observer_error)]]);
facts('environment-facts',[['Commit',`${S.metadata.commit}（dirty=${S.metadata.repository_dirty}）`],['二进制 SHA-256',S.metadata.binary_sha256],['CPU',`${S.metadata.cpu} · ${S.metadata.logical_cpus} logical CPUs`],['内存',`${fmt(S.metadata.memory_total_gib,1)} GiB`],['平台',S.metadata.platform],['图表库',`${S.metadata.chart_library}（源码内嵌，无 CDN）`]]);document.getElementById('footer').innerHTML=`生成于 ${S.metadata.generated_at} · 原始数据 <a href="__RESULT_HREF__">result.json</a> · 机器可读摘要 <a href="summary.json">summary.json</a>`;
</script></body></html>'''


def write_stress_report(
    result_path: Path,
    output_path: Path,
    *,
    d3_path: Path = DEFAULT_D3,
) -> dict:
    """Create an offline interactive report and return its compact summary."""
    if result_path.resolve() == output_path.resolve():
        raise ValueError("report output must not overwrite the input result")
    payload = json.loads(result_path.read_text(encoding="utf-8"))
    summary, report = build_report(payload, d3_path.read_text(encoding="utf-8"))
    output_path.parent.mkdir(parents=True, exist_ok=True)
    result_href = html.escape(
        quote(Path(os.path.relpath(result_path.resolve(), output_path.parent.resolve())).as_posix()),
        quote=True,
    )
    report = report.replace("__RESULT_HREF__", result_href)
    (output_path.parent / "summary.json").write_text(
        json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    output_path.write_text(report, encoding="utf-8")
    return summary
