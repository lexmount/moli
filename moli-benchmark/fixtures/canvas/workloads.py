"""Canvas 2D performance workload definitions for the M0 baseline.

Each workload is a JS source snippet executed inside a canvas-less harness that
defines `N` (operation count) and `SIZE` (canvas edge in px), creates a canvas
via `makeCanvas()`, obtains a 2D context via `getCtx(canvas)`, runs the workload,
and returns a JSON object of the form:

    { "workload": name, "size": SIZE, "ops": N,
      "recordMs": ..., "flushMs": ..., "totalMs": ... }

The harness records timing at the boundaries the architecture cares about:
"record" is the cumulative time of the draw-call loop (input/recording), "flush"
is the time of the single pixel observation that forces the recording to execute,
and "total" is wall time of the whole workload including observations.

Workload list mirrors proposal section 11.1.
"""

from __future__ import annotations


def path_fill(body: str) -> str:
    """Many small path fills followed by ONE readback."""
    return f"""
      const before = performance.now();
      for (let i = 0; i < N; i++) {{
        {body}
      }}
      const recordMs = performance.now() - before;
      const beforeFlush = performance.now();
      const probe = ctx.getImageData(0, 0, 1, 1).data[0];
      const flushMs = performance.now() - beforeFlush;
      return JSON.stringify({{ workload: 'path_fill', size: SIZE, ops: N, recordMs, flushMs, totalMs: recordMs + flushMs, probe }});
    """


def path_stroke() -> str:
    return f"""
      const before = performance.now();
      for (let i = 0; i < N; i++) {{
        ctx.beginPath();
        ctx.moveTo(i % SIZE + 10, 10 + (i % 20));
        ctx.lineTo((i + 30) % SIZE + 10, 40 + (i % 20));
        ctx.stroke();
      }}
      const recordMs = performance.now() - before;
      const beforeFlush = performance.now();
      const probe = ctx.getImageData(0, 0, 1, 1).data[0];
      const flushMs = performance.now() - beforeFlush;
      return JSON.stringify({{ workload: 'path_stroke', size: SIZE, ops: N, recordMs, flushMs, totalMs: recordMs + flushMs, probe }});
    """


def rect_draws() -> str:
    return f"""
      const before = performance.now();
      for (let i = 0; i < N; i++) {{
        ctx.fillRect((i * 7) % SIZE, (i * 13) % SIZE, 8, 8);
        ctx.strokeRect((i * 5) % SIZE, (i * 11) % SIZE, 8, 8);
      }}
      const recordMs = performance.now() - before;
      const beforeFlush = performance.now();
      const probe = ctx.getImageData(0, 0, 1, 1).data[0];
      const flushMs = performance.now() - beforeFlush;
      return JSON.stringify({{ workload: 'rect', size: SIZE, ops: N, recordMs, flushMs, totalMs: recordMs + flushMs, probe }});
    """


def text_draws() -> str:
    return f"""
      ctx.font = '16px sans-serif';
      const before = performance.now();
      for (let i = 0; i < N % 200; i++) {{
        ctx.fillText('Moli', (i * 9) % SIZE, 20 + (i % 40));
      }}
      const recordMs = performance.now() - before;
      const beforeFlush = performance.now();
      const probe = ctx.getImageData(0, 0, 1, 1).data[0];
      const flushMs = performance.now() - beforeFlush;
      return JSON.stringify({{ workload: 'text', size: SIZE, ops: N, recordMs, flushMs, totalMs: recordMs + flushMs, probe }});
    """


def draw_image() -> str:
    return f"""
      const img = document.createElement('canvas'); img.width = img.height = 32;
      const imgCtx = img.getContext('2d'); imgCtx.fillStyle = 'red'; imgCtx.fillRect(0,0,32,32);
      const before = performance.now();
      for (let i = 0; i < N % 500; i++) {{
        ctx.drawImage(img, (i * 3) % SIZE, (i * 5) % SIZE);
      }}
      const recordMs = performance.now() - before;
      const beforeFlush = performance.now();
      const probe = ctx.getImageData(0, 0, 1, 1).data[0];
      const flushMs = performance.now() - beforeFlush;
      return JSON.stringify({{ workload: 'draw_image', size: SIZE, ops: N, recordMs, flushMs, totalMs: recordMs + flushMs, probe }});
    """


def draw_clear_write_mix() -> str:
    return f"""
      const id = ctx.createImageData(16, 16);
      const before = performance.now();
      for (let i = 0; i < N; i++) {{
        ctx.fillRect((i * 9) % SIZE, (i * 17) % SIZE, 16, 16);
        if (i % 3 === 0) ctx.clearRect((i * 9) % SIZE, (i * 17) % SIZE, 8, 8);
        if (i % 5 === 0) ctx.putImageData(id, (i * 3) % SIZE, (i * 7) % SIZE);
      }}
      const recordMs = performance.now() - before;
      const beforeFlush = performance.now();
      const probe = ctx.getImageData(0, 0, 1, 1).data[0];
      const flushMs = performance.now() - beforeFlush;
      return JSON.stringify({{ workload: 'draw_clear_write', size: SIZE, ops: N, recordMs, flushMs, totalMs: recordMs + flushMs, probe }});
    """


def readback_every_draw() -> str:
    return f"""
      const before = performance.now();
      let acc = 0;
      for (let i = 0; i < N; i++) {{
        ctx.fillRect((i * 7) % SIZE, (i * 11) % SIZE, 4, 4);
        acc += ctx.getImageData(0, 0, 1, 1).data[0];
      }}
      const totalMs = performance.now() - before;
      return JSON.stringify({{ workload: 'readback_every_draw', size: SIZE, ops: N, recordMs: totalMs, flushMs: 0, totalMs, acc }});
    """


def repeated_clean_reads() -> str:
    return f"""
      ctx.fillStyle = '#ff0000'; ctx.fillRect(0, 0, SIZE, SIZE);
      const before = performance.now();
      let acc = 0;
      for (let i = 0; i < Math.min(N, 100); i++) {{
        acc += ctx.getImageData(0, 0, 1, 1).data[0];
      }}
      const totalMs = performance.now() - before;
      return JSON.stringify({{ workload: 'repeated_clean_reads', size: SIZE, ops: N, recordMs: 0, flushMs: 0, totalMs, acc }});
    """


def self_draw() -> str:
    return f"""
      const before = performance.now();
      for (let i = 0; i < Math.min(N, 50); i++) {{
        ctx.drawImage(canvas, (i * 3) % SIZE, (i * 5) % SIZE);
      }}
      const recordMs = performance.now() - before;
      const beforeFlush = performance.now();
      const probe = ctx.getImageData(0, 0, 1, 1).data[0];
      const flushMs = performance.now() - beforeFlush;
      return JSON.stringify({{ workload: 'self_draw', size: SIZE, ops: N, recordMs, flushMs, totalMs: recordMs + flushMs, probe }});
    """


WORKLOADS: dict[str, str] = {
    "path_fill": path_fill(
        "ctx.beginPath(); ctx.rect((i*11)%SIZE, (i*7)%SIZE, 12, 12); ctx.fill();"
    ),
    "path_stroke": path_stroke(),
    "rect": rect_draws(),
    "text": text_draws(),
    "draw_image": draw_image(),
    "draw_clear_write": draw_clear_write_mix(),
    "readback_every_draw": readback_every_draw(),
    "repeated_clean_reads": repeated_clean_reads(),
    "self_draw": self_draw(),
}


def html_for_workload(name: str, size: int, ops: int) -> str:
    body = WORKLOADS[name]
    return (
        "<!doctype html><meta charset=utf-8><title>moli canvas baseline</title>"
        f"<script>\n"
        f"(() => {{\n"
        f"  const SIZE = {size}; const N = {ops};\n"
        f"  const canvas = document.createElement('canvas');\n"
        f"  canvas.width = canvas.height = SIZE;\n"
        f"  const ctx = canvas.getContext('2d');\n"
        f"  ctx.fillStyle = ctx.strokeStyle = '#ff0000';\n"
        f"  ctx.lineWidth = 2;\n"
        f"  document.title = 'canvas-' + \"{name}\" + '-' + SIZE + '-' + N;\n"
        f"  globalThis.__canvasResult = (() => {{\n{body}\n  }})();\n"
        f"}})()\n"
        f"</script>"
    )
