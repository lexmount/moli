# /// script
# requires-python = ">=3.11"
# dependencies = ["pymupdf==1.28.2"]
# ///
"""Exercise the real CLI and a PDF reader, including ActualText and Type3 glyphs.

Run after building Moli:
    uv run scripts/check-native-pdf.py --moli target/debug/moli
Requires Poppler's pdftoppm (poppler-utils).
Unlike lopdf's basic extractor, MuPDF interprets ActualText, reversed text and
colored Type3 glyphs. Keep these reader checks separate from the Rust structural
and plain-text regressions. Artifacts and observed process costs are retained.
"""

import argparse
import base64
from collections import Counter
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import shutil
import subprocess
import threading
import time

import pymupdf

ROOT = Path(__file__).resolve().parents[1]


def data_url(data, media_type):
    return f"data:{media_type};base64," + base64.b64encode(data).decode()


def capture(binary, output, name, html, url=None):
    path = output / f"{name}.pdf"
    started = time.perf_counter()
    peak_kib = 0
    with path.open("wb") as target:
        process = subprocess.Popen(
            [str(binary), "fetch", "--resource", "--layout", "--wait-until", "load",
             "--log-level", "error", "--http-no-proxy", "*", "--dump", "pdf",
             url or data_url(html.encode(), "text/html")],
            stdout=target, stderr=subprocess.PIPE,
        )
        while process.poll() is None:
            if time.perf_counter() - started > 120:
                process.kill()
                process.communicate()
                raise TimeoutError(f"{name} exceeded 120 seconds")
            try:
                for line in Path(f"/proc/{process.pid}/status").read_text().splitlines():
                    if line.startswith("VmHWM:"):
                        peak_kib = max(peak_kib, int(line.split()[1]))
            except FileNotFoundError:
                pass
            time.sleep(0.005)
        stderr = process.communicate()[1].decode(errors="replace")
    if process.returncode:
        raise RuntimeError(f"{name} failed: {stderr}")
    seconds = time.perf_counter() - started
    document = pymupdf.open(path)
    text = " ".join(" ".join(page.get_text().split()) for page in document)
    (output / f"{name}.txt").write_text(text)
    document[0].get_pixmap(matrix=pymupdf.Matrix(1.5, 1.5)).save(output / f"{name}.png")
    links = [{"page": page.number, "kind": link["kind"],
              "rect": list(link["from"]), "uri": link.get("uri"),
              "destination_page": link.get("page"),
              "destination_point": list(link["to"]) if "to" in link else None}
             for page in document for link in page.get_links()]
    (output / f"{name}-links.json").write_text(json.dumps(links, indent=2) + "\n")
    info = {"pages": len(document), "pdf_bytes": path.stat().st_size,
            "seconds": round(seconds, 4), "peak_rss_kib_sampled": peak_kib,
            "fonts": sorted({font[3] for page in document for font in page.get_fonts()}),
            "image_count": sum(len(page.get_images()) for page in document),
            "link_count": len(links)}
    document.close()
    return text, info


def screenshot(binary, output, name, html, url=None):
    path = output / f"{name}-screenshot.png"
    result = subprocess.run(
        [str(binary), "fetch", "--resource", "--layout", "--wait-until", "load",
         "--log-level", "error", "--http-no-proxy", "*", "--dump", "screenshot",
         url or data_url(html.encode(), "text/html")],
        capture_output=True, check=True, timeout=120,
    )
    path.write_bytes(result.stdout)
    return pymupdf.Pixmap(path)


def check_button_backgrounds(binary, output, html):
    reference = screenshot(binary, output, "backgrounds", html)
    with pymupdf.open(output / "backgrounds.pdf") as document:
        page = document[0]
        # CLI PDF fits the viewport width inside the default 1 cm margins.
        margin = 72 / 2.54
        scale = (page.rect.width - 2 * margin) / reference.width
        clip = pymupdf.Rect(margin, margin, page.rect.width - margin,
                            margin + reference.height * scale)
        rendered = page.get_pixmap(matrix=pymupdf.Matrix(1 / scale, 1 / scale), clip=clip)
        rendered.save(output / "backgrounds-rendered.png")
        for x, y in [(110, 112), (320, 112), (460, 112)]:
            expected = reference.pixel(x, y)[:3]
            actual = rendered.pixel(round(margin / scale + x - rendered.x),
                                    round(margin / scale + y - rendered.y))[:3]
            assert min(expected) < 180, ("missing screenshot background", x, y, expected)
            assert max(abs(a - b) for a, b in zip(actual, expected)) <= 3, (
                "PDF button background differs from screenshot", x, y, actual, expected
            )
        # A correct background must also leave the white labels visible. Check
        # each label's interior separately from its extractable Unicode text.
        for x0, x1 in [(115, 265), (310, 470)]:
            white = 0
            for y in range(121, 140):
                for x in range(x0, x1):
                    pixel = rendered.pixel(round(margin / scale + x - rendered.x),
                                           round(margin / scale + y - rendered.y))[:3]
                    white += min(pixel) > 235
            assert white > 80, ("white button label is not visible", x0, white)


@contextmanager
def fixture_server(resources):
    """Serve only fixed in-memory documents; no public web resources are fetched."""
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            body = resources.get(self.path)
            self.send_response(200 if body is not None else 404)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.end_headers()
            self.wfile.write((body or "Missing fixture").encode())

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def check_links(binary, output, style):
    # Explicit geometry avoids dependence on system fonts and viewport guesses.
    # Text uses bundled fonts; the child uses a vector icon without font lookup.
    html = style + """<base href="/fixtures/"><style>
body{margin:0;height:3600px} a{color:#174ac0}
.box{position:absolute;display:block;width:100px;height:30px;line-height:30px}
#relative{left:40px;top:40px;background:#c2d8ff}
#wrap{position:absolute;left:40px;top:100px;width:100px;line-height:30px}
#rotate{left:300px;top:100px;transform:rotate(30deg);transform-origin:0 0;background:#c2d8ff}
#clip{position:absolute;left:40px;top:230px;width:70px;height:30px;overflow:hidden}
#clip a{display:block;width:150px;height:30px;background:#c2d8ff}
#internal{left:40px;top:300px} #named{left:170px;top:300px}
#destination{position:absolute;left:40px;top:3200px;width:300px;height:30px;background:#5ac88c}
iframe{position:absolute;left:300px;top:230px;width:180px;height:90px;border:0}
#hidden{opacity:0;left:500px;top:40px}
#spanning{position:absolute;left:600px;top:1600px;width:80px;height:1600px;background:#d0e6f8}
</style>
<a class=box id=relative href="../target?q=1&amp;x=2">Relative</a>
<div id=wrap><a href="https://links.example/wrapped">First Second</a></div>
<a class=box id=rotate href="https://links.example/rotated">Rotate</a>
<div id=clip><a href="https://links.example/clipped">Clipped text</a></div>
<a class=box id=internal href="/index.html#destination">Internal</a>
<a class=box id=named href="/index.html#old%20anchor">Named</a>
<a class=box id=hidden href="https://links.example/hidden">Hidden</a>
<a id=spanning href="https://links.example/paged"></a>
<iframe src="child.html"></iframe>
<div id=destination>Destination<a name="old anchor"></a></div>"""
    child = """<style>body{margin:0;font-size:0}a{position:absolute;left:10px;top:10px;
display:block;width:100px;height:30px;background:#c2d8ff}</style><a href="child-target">
<svg width="20" height="20" viewBox="0 0 20 20"><path d="M5 5H15V15H5Z" fill="#174ac0"/></svg></a>"""
    with fixture_server({"/index.html": html, "/fixtures/child.html": child}) as origin:
        text, info = capture(binary, output, "links", html, origin + "/index.html")
        reference = screenshot(binary, output, "links", html, origin + "/index.html")
        with pymupdf.open(output / "links.pdf") as document:
            page = document[0]
            margin = 72 / 2.54
            scale = (page.rect.width - margin * 2) / reference.width
            links = page.get_links()
            by_uri = {}
            for link in links:
                if link["kind"] == pymupdf.LINK_URI:
                    by_uri.setdefault(link["uri"], []).append(link)
            assert set(by_uri) == {origin + "/target?q=1&x=2",
                                  "https://links.example/wrapped",
                                  "https://links.example/rotated",
                                  "https://links.example/clipped",
                                  "https://links.example/paged",
                                  origin + "/fixtures/child-target"}, by_uri

            def css_rect(link):
                rect = link["from"]
                return pymupdf.Rect(*[(value - margin) / scale for value in rect])

            def near(actual, expected, tolerance=1):
                assert max(abs(a - b) for a, b in zip(actual, expected)) <= tolerance, (actual, expected)

            near(css_rect(by_uri[origin + "/target?q=1&x=2"][0]), (40, 40, 140, 70))
            near(css_rect(by_uri["https://links.example/clipped"][0]), (40, 230, 110, 260))
            near(css_rect(by_uri[origin + "/fixtures/child-target"][0]), (310, 240, 410, 270))
            wrapped = by_uri["https://links.example/wrapped"]
            assert len(wrapped) == 2, wrapped
            assert all(css_rect(link).height < 31 for link in wrapped), wrapped
            near(css_rect(by_uri["https://links.example/rotated"][0]),
                 (285, 100, 386.60, 175.98), tolerance=1.5)
            # MuPDF exposes annotation rectangles; inspect QuadPoints as well
            # so a rotated link cannot silently degrade to its bounding box.
            rotated = by_uri["https://links.example/rotated"][0]
            kind, values = document.xref_get_key(rotated["xref"], "QuadPoints")
            assert kind == "array" and len(values.strip("[]").split()) == 8, values
            points = [float(v) for v in values.strip("[]").split()]
            assert abs(points[1] - points[3]) > 10, points
            rendered = page.get_pixmap(matrix=pymupdf.Matrix(1 / scale, 1 / scale))
            rendered.save(output / "links-rendered.png")
            # Interior samples couple visible content to the independently
            # asserted annotation regions, including rotation, clip and iframe.
            for x, y in [(45, 45), (45, 250), (120, 250), (316, 245), (302, 150)]:
                actual = rendered.pixel(round(margin / scale + x), round(margin / scale + y))[:3]
                expected = reference.pixel(x, y)[:3]
                assert max(abs(a - b) for a, b in zip(actual, expected)) <= 3, (
                    "linked content differs from screenshot", x, y, actual, expected)
            assert min(reference.pixel(45, 45)[:3]) < 220
            assert min(reference.pixel(120, 250)[:3]) > 250, "overflow clip must hide content"
            internal = [link for link in links if link["kind"] == pymupdf.LINK_GOTO]
            destination_page = int(3200 * scale / (page.rect.height - margin * 2))
            assert destination_page > 0
            assert len(internal) == 2 and all(link["page"] == destination_page for link in internal), internal
            target = next(link for link in internal if abs(link["to"].x - (margin + 40 * scale)) < 1)
            near(target["to"], (margin + 40 * scale,
                               margin + 3200 * scale - destination_page * (page.rect.height - margin * 2)))
            assert all(margin <= link["to"].x <= margin + 340 * scale for link in internal), internal
            assert all("hidden" not in str(link) for page in document for link in page.get_links())
            assert len(document) == 2
            page_height = (page.rect.height - margin * 2) / scale
            near(css_rect(by_uri["https://links.example/paged"][0]), (600, 1600, 680, page_height))
            second = document[1].get_links()
            assert len(second) == 1 and second[0].get("uri") == "https://links.example/paged", second
            near(css_rect(second[0]), (600, 0, 680, 3200 - page_height))
            preview = document[1].get_pixmap()
            preview.save(output / "links-page-2.png")
            x, y = second[0]["from"].tl
            assert max(abs(a - b) for a, b in zip(preview.pixel(round(x + 3), round(y + 3))[:3],
                                                 (208, 230, 248))) <= 1
        assert "Destination" in text and "Hidden" not in text, text
        assert all("MoliPDFTest" in font for font in info["fonts"]), info["fonts"]
    return info


def check_translucent_colr(binary, output, pdftoppm):
    """Inspect actual pixels: Type3 CharProcs can override a caller's ca=0."""
    results = {}
    reference = {}
    for svg in [False, True]:
        suffix = "-svg" if svg else ""
        font = data_url((ROOT / "moli-paint/tests/fixtures" /
                         f"moli-text-stroke-colr{suffix}.ttf").read_bytes(), "font/ttf")
        for weight in [400, 700]:
            for order in ["normal", "stroke fill"]:
                for label, alpha, filtered in [
                    ("AA", 0, False), ("AA", 1, False),
                    ("AB", 0, False), ("AB", 1, False),
                    ("AA", 0, True),
                ]:
                    name = f"colr{suffix}-{weight}-{order.replace(' ', '-')}-{label}-{alpha}-{filtered}"
                    html = f"""<style>
@font-face{{font-family:Color;src:url({font});font-weight:400}}
body{{margin:0;background:white}}
div{{position:absolute;left:60px;top:60px;font:100px/1 Color;font-weight:{weight};
color:rgba(0,0,255,{alpha});-webkit-text-stroke:4px blue;paint-order:{order};
filter:{'opacity(.5)' if filtered else 'none'}}}
</style><div>{label}</div>"""
                    text, info = capture(binary, output, name, html)
                    assert text == label, (name, text)
                    expected = 127
                    if filtered:
                        # The filter already contains CPU-rendered artwork.
                        # PDF semantics must not paint over that image. Color
                        # stroke support in the CPU renderer is a separate task.
                        image = screenshot(binary, output, name, html)
                        rgb = image.samples
                        red = Counter(tuple(rgb[i:i + 3]) for i in range(0, len(rgb), image.n)
                                      if rgb[i] == 255 and rgb[i + 1] == rgb[i + 2] < 250)
                        expected = red.most_common(1)[0][0][1]
                    target = output / f"{name}-poppler"
                    subprocess.run([str(pdftoppm), "-r", "144", "-singlefile", "-png",
                                    str(output / f"{name}.pdf"), str(target)],
                                   check=True, capture_output=True)
                    pixmaps = {"Poppler": pymupdf.Pixmap(str(target) + ".png")}
                    with pymupdf.open(output / f"{name}.pdf") as document:
                        pixmaps["MuPDF"] = document[0].get_pixmap(matrix=pymupdf.Matrix(2, 2))
                    colors = {}
                    for reader, pixmap in pixmaps.items():
                        rgb = pixmap.samples
                        red = Counter(tuple(rgb[i:i + 3]) for i in range(0, len(rgb), pixmap.n)
                                      if rgb[i] == 255 and rgb[i + 1] == rgb[i + 2] < 250)
                        assert red, (name, reader, "missing COLR layer")
                        interior, pixels = red.most_common(1)[0]
                        assert max(abs(channel - expected) for channel in interior[1:]) <= 1, (
                            name, reader, interior, "PDF must not composite extra COLR artwork")
                        assert min(color[1] for color in red) >= expected - 1, (
                            name, reader, "extra translucent painting")
                        assert pixels > 100, (name, reader, "missing COLR interior")
                        colors[reader] = list(interior)
                        # An unused SVG record and synthetic weight must not
                        # change the COLR glyph's palette opacity.
                        key = (reader, label, alpha, order, filtered)
                        if key in reference:
                            assert max(abs(a - b) for a, b in zip(interior, reference[key])) <= 1, (
                                name, reader, interior, reference[key])
                        else:
                            reference[key] = interior
                    results[name] = {**info, "interior_rgb": colors}
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--moli", type=Path, default=ROOT / "target/debug/moli")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target/native-pdf-check")
    parser.add_argument("--pdftoppm", type=Path, default=shutil.which("pdftoppm"),
                        help="Poppler renderer required for translucent Type3 regressions")
    args = parser.parse_args()
    if args.pdftoppm is None or not args.pdftoppm.is_file():
        parser.error("pdftoppm is required; install poppler-utils or use --pdftoppm")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    font = data_url((ROOT / "moli-paint/tests/fixtures/moli-pdf.ttf").read_bytes(), "font/ttf")
    cjk = data_url((ROOT / "moli-layout/tests/fixtures/moli-cjk.ttf").read_bytes(), "font/ttf")
    style = f"""<meta charset=utf-8><style>
@font-face{{font-family:PDFTest;src:url({font});font-weight:400 900}}
@font-face{{font-family:CJK;src:url({cjk})}}
body{{font:20px PDFTest,CJK;margin:24px}} p{{margin:16px 0}}
</style>"""
    results = {}
    text, results["minimal"] = capture(args.moli.resolve(), args.output_dir, "minimal",
                                        style + "<h1>Font Text 123</h1>")
    assert text == "Font Text 123", text
    assert results["minimal"]["image_count"] == 0
    assert any("+MoliPDFTest" in font for font in results["minimal"]["fonts"])

    html = style + """<style>
button{position:absolute;left:100px;top:100px;width:180px;height:60px;
color:white;background:#325dff;border:0;border-radius:12px;font:20px PDFTest}
#gradient{left:300px;background:linear-gradient(90deg in srgb,#325dff,#9864ff)}
</style><button>Search</button><button id=gradient>Gradient</button>"""
    text, results["backgrounds"] = capture(args.moli.resolve(), args.output_dir, "backgrounds", html)
    assert text == "Search Gradient", text
    check_button_backgrounds(args.moli.resolve(), args.output_dir, html)

    results["links"] = check_links(args.moli.resolve(), args.output_dir, style)

    expected = ["Font Text 123 中文", "office fi ffi á", "Second page a", "abc אבג xyz",
                "Color ★", "Weight 650", "Italic"]
    html = style + """<p>Font Text 123 中文</p><p>office fi ffi á</p>
<p>Second page a</p><p>abc אבג xyz</p><p>Color ★</p>
<p style="font-weight:650">Weight 650</p><p style="font-style:italic">Italic</p>"""
    text, results["shaping"] = capture(args.moli.resolve(), args.output_dir, "shaping", html)
    assert text == " ".join(expected), text

    html = style + """<p style="opacity:.65">Opacity text</p>
<p style="text-shadow:2px 2px 2px red">Shadow text once</p>
<p style="filter:blur(.3px)">Filtered text once</p>
<p style="opacity:0">Hidden opacity</p>
<p style="mask-image:linear-gradient(transparent,transparent)">Hidden mask</p>"""
    text, results["effects"] = capture(args.moli.resolve(), args.output_dir, "effects", html)
    for phrase in ["Opacity text", "Shadow text once", "Filtered text once"]:
        assert text.count(phrase) == 1, text
    assert "Hidden" not in text, text

    # Exceeds the full-document screenshot CSS dimension limit. There is no
    # full-document pixel allocation in this path, even at the default viewport.
    html = style + '<main style="height:200100px">' + "".join(
        f'<p style="position:absolute;top:{i * 700}px">Row {i:03}</p>' for i in range(286)
    ) + "</main>"
    text, results["long_document"] = capture(args.moli.resolve(), args.output_dir, "long-document", html)
    assert text == " ".join(f"Row {i:03}" for i in range(286)), text
    assert results["long_document"]["pages"] > 1
    assert results["long_document"]["image_count"] == 0
    for name, info in results.items():
        assert all(font.split("+")[-1] in {"MoliPDFTest", "DroidSansFallback"}
                   for font in info["fonts"]), ("unexpected system font", name, info["fonts"])
    color_stroke = check_translucent_colr(args.moli.resolve(), args.output_dir, args.pdftoppm)
    report = {"binary": str(args.moli.resolve()), "binary_bytes": args.moli.stat().st_size,
              "reader": pymupdf.version[0], "cases": results, "translucent_colr": color_stroke}
    (args.output_dir / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
