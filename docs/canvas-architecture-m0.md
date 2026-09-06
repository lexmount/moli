# Canvas 2D — M0 Baseline, Ownership Note, and Migration Inventory

Status: implementation baseline (M0), not a description of completed work.

This document is the first deliverable of the Canvas 2D re-architecture covered by
`canvas-2d-proposal.en.md`. It records the **current** (pre-migration) pixel
ownership, every read/write/reset/retirement entry point, a reproducible baseline,
and the routing checklist used to review the final integration. It is intentionally
a snapshot of reality, not of the target design.

Reference revision for this note: branch `canvas_2D_moli`, commit `4d6e0373`.

---

## 1. Building blocks / terminology

| Term | Meaning in this project |
| --- | --- |
| Backing store | The V8 `Uint8ClampedArray` private slot on a canvas-like object that holds the writable RGBA8 pixels that all 2D operations mutate |
| CanvasResourceStore | `HashMap<DomHandle, Arc<moli_image::RgbaImage>>` in `moli-renderer-v8/src/native_bridge/context_host/canvas_resources.rs`; the published, immutable, page-visible image per HTML canvas |
| Context state | V8 private-slot strings/numbers/bools on the context object: `fillStyle`, `font`, `globalAlpha`, `globalCompositeOperation`, `lineWidth`, `lineCap`, `lineJoin`, `miterLimit`, `lineDashOffset`, `lineDash`, `strokeStyle`, `imageSmoothingEnabled`, `imageSmoothingQuality` |
| Path state | `Canvas2dPathState` held per context in a weak-keyed store (`canvas/state.rs`), reclaimed by GC / isolate teardown |
| Rasterization | `moli_paint::raster_snapshot(&PaintSnapshot)`, invoked per draw by `rasterize_canvas_fragment()` in `canvas/context2d.rs` |
| Snapshot | `moli_image::RgbaImage` (straight-alpha RGBA8); the immutable unit pages and `getImageData`-adjacent consumers use |

---

## 2. Current pixel ownership (two writable/observable planes today)

There are **two** pixel stores that must be kept in sync, which is the core
redundancy the project removes:

1. **Mutable backing store** — `CANVAS_BACKING_STORE_SLOT` (`__moliCanvasBackingStore`), a
   `Uint8ClampedArray` owned by the canvas-like JS object. All 2D drawing and
   `putImageData` mutate a **fresh byte copy** of this view, then write it back.
   - Managed by `backing_store.rs`: `with_canvas_like_pixels_mut()`, `canvas_like_pixels_copy()`, `ensure_canvas_like_backing_store()`.
   - It is straight-alpha RGBA8, row major, dimensions from the width/height slots.

2. **Published page image** — `CanvasResourceStore.pixels_by_element: HashMap<DomHandle, Arc<RgbaImage>>`.
   - `replace_canvas_pixels(handle, w, h, rgba)` replaces the whole stored
     `Arc<RgbaImage>` on every mutation, bumps `VisualResourceGeneration`, and
     enforces `MAX_RETAINED_CANVAS_PAINT_BYTES` (256 MiB).
   - Page painting reads it via `JsContextHost::canvas_pixels_for_layout()` →
     `source_view.rs` → `LayoutImageResource`. This is an **immutable snapshot** plane
     already; it is not copied per pixel-read.

The renderer's `rasterize_canvas_fragment()` performs, per-path-draw:
  copy backing view -> Vec, build a fresh `PaintSnapshot` covering the **whole
  canvas**, run `moli_paint::raster_snapshot` (allocates a new `VelloCpuImageRenderer`
  and a full RgbaImage), `composite_rgba8_over` the premultiplied result into the
  straight copy, write back, then `replace_canvas_pixels(...)` with the full copy.

Other ordinary draws (`fillRect`, `clearRect`, `fillText`/`strokeText`, `drawImage`)
go through the same full-copy `with_canvas_like_pixels_mut` path but write pixels
directly instead of building a `PaintSnapshot`. `putImageData` writes raw pixels
directly (correct raw overwrite semantics) via `blit_image_data`.

Consequence: cost scales with canvas area for hundreds of small operations because
every operation copies the whole plane and (for paths) allocates a fresh backend + full
raster.

---

## 3. Entry-point inventory (complete routing checklist)

Legend — **D** = ordinary draw, **W** = explicit pixel write, **R** = pixel read/
observation, **S** = state/geometry (no pixel work), **RST** = reset/dimension,
**STUB** = existing incomplete implementation, **OOB** = out of scope for this
project.

### 3.1 CanvasRenderingContext2D / OffscreenCanvasRenderingContext2D — `canvas/context2d.rs`

| # | Entry point (fn/line) | Kind | Route today | Target stage |
|---|---|---|---|---|
| 1 | fillRect (700) | D | full-copy `with_canvas_like_pixels_mut` → `paint_rect` | M2 D record |
| 2 | clearRect (719) | D | full-copy → `paint_rect` `[0,0,0,0]` (destructive) | M2 record w/ ordered clear |
| 3 | fill / stroke (1066/1094) | D | `PaintSnapshot` via `rasterize_canvas_fragment` | M4 recorded path |
| 4 | strokeRect (1123) | D | builds temp rect path, same fragment route | M4 recorded |
| 5 | fillText / strokeText (1572/1586) | D | `with_canvas_like_pixels_mut` → `draw_text` (font8x8) | M1–M4 recorded text |
| 6 | drawImage (1618) | D | `html_image_pixels_copy`/`canvas_like_pixels_copy` then `blit_draw_image_filtered` | M4 recorded + source snapshot |
| 7 | putImageData (1856) | W | full-copy → `blit_image_data` (raw overwrite) | M2/M4 ordered pixel-write boundary |
| 8 | getImageData (1921) | R | `canvas_like_pixels_copy` → `extract_image_data` | M2/M5 flush+readback |
| 9 | measureText (1815) | S | `measure_text_width` | S |
| 10 | createImageData (1834) | S | alloc empty ImageData | S |
| 11 | isPointInPath (1564) | STUB | always returns `false` | STUB/OOB |
| 12 | createLinearGradient (1766) | STUB | returns inert object; `addColorStop` validates offset only, no rendering | STUB/OOB |
| 13 | setLineDash/getLineDash (1690/1742) | S | V8-array slot | S |
| 14 | noop (1683) | STUB | no-op | STUB |
| 15 | path builders (rect, beginPath, closePath, moveTo, lineTo, quadraticCurveTo, bezierCurveTo, arc, arcTo, ellipse) | S | `Canvas2dPathState` | M1 → `moli-canvas::path` |
| 16 | transform (translate/scale/rotate/transform/setTransform/resetTransform) | S | `Canvas2dPathState.transform` | M1 → `moli-canvas::context` |
| 17 | state setters/getters (fillStyle, strokeStyle, font, lineWidth/Cap/Join, miterLimit, lineDashOffset, globalAlpha, globalCompositeOperation, imageSmoothing*) | S | V8 private slots | M1 state → `moli-canvas::context` |
| 18 | reset_canvas_context_state (29) | RST | re-init slots + reset path | M4 reset |
| 19 | rasterize_canvas_fragment (1512) | (impl) | per-path full-frame page pipeline | M6 delete |
| 20 | composite_rgba8_over (1538) | (impl) | premult→straight composite helper | M2 replace w/ format module |

### 3.2 Backing store / pixels — `canvas/backing_store.rs`

| # | Entry point | Kind | Notes | Target |
|---|---|---|---|---|
| 21 | attach_canvas_like_context_object | init | links context↔canvas, initializes backing store | M3 ownership |
| 22 | canvas_2d_context (74) | R | returns stored 2D context object | M3 |
| 23 | canvas_owner_from_context (115) | R | reverse context→canvas | M3 |
| 24 | with_canvas_like_pixels_mut (122) | W | **full-copy mutate+write-back** (major copy source) | M6 remove for draws; keep for putImageData boundary until M4 |
| 25 | canvas_like_pixels_copy (144) | R | **full copy** (major copy source for drawImage/getImageData/page) | M6 replace with snapshot readback |
| 26 | reset_canvas_like_backing_store / reset_html_canvas_backing_store_for_dimension_assignment | RST | zero-fills backing store on dimension change | M4/M6 |
| 27 | canvas_like_to_data_url (93) | R | full copy → `encode_data_url` | M5 flush+encode |
| 28 | ensure_canvas_like_backing_store | alloc | lazily creates/zeros backing view | M3 surface |

### 3.3 Canvas element (browser-facing) — `native_bridge/element/canvas.rs`, `context_bootstrap/canvas.rs`

| # | Entry point | Kind | Notes | Target |
|---|---|---|---|---|
| 29 | HTMLCanvasElement width/height setters | RST | set reflected attribute → resets backing store + context | M3/M4 resize semantics |
| 30 | HTMLCanvasElement width/height getters | S | default 300×150 | S |
| 31 | getContext (`CanvasContextKind`) | init | returns cached context object per kind slot | M3 identity |
| 32 | toDataURL | R | `canvas_like_to_data_url` | M5 flush+encode |
| 33 | build HTML/Offscreen canvas objects | init | constructors | M3 |

### 3.4 OffscreenCanvas — `canvas/offscreen.rs`

| # | Entry point | Kind | Notes | Target |
|---|---|---|---|---|
| 34 | OffscreenCanvas width/height setters | RST | reset backing store | M3/M4 |
| 35 | getContext | init | builds 2D/WebGL context, attaches | M3 |
| 36 | convertToBlob (172) | STUB | resolves a blob of empty bytes | STUB/OOB |
| 37 | 2D context constructor, object init | init | `canvas_rendering_context_2d_constructor_callback` etc. | M3 |

### 3.5 Publication and page painting

| # | Entry point | Kind | Notes | Target |
|---|---|---|---|---|
| 38 | CanvasResourceStore.replace/remove/get | R | replaces entire `Arc<RgbaImage>` per change | M3 publish via snapshot |
| 39 | retire_canvas_resources_for_document | RST | removes images whose owner document retired, but *resolves ownership at retirement* to preserve adopted canvases | M3 lifecycle |
| 40 | canvas_pixels_for_layout → source_view.rs | R | page painting reads immutable `Arc<RgbaImage>` | M5 read snapshot |
| 41 | VisualResourceGeneration bump | RST | marks page dirty; drives repaint/screencast | M5 invalidation on record |

---

## 4. Read / write / reset / retirement matrix

**Pixel owners today**
- Mutable: V8 `Uint8ClampedArray` (canvas-like object).
- Immutable published: `CanvasResourceStore` `Arc<RgbaImage>` (per HTML canvas).

**Every place pixels are read or written**
- Write (full-plane copy): fillRect, clearRect, fillText, strokeText, drawImage, putImageData (raw), path fill/stroke (via snapshot+composite).
- Read: getImageData, drawImage(source=canvas), toDataURL, page painting (canvas_pixels_for_layout), screencast (through page painting), clone/copy in `canvas_like_pixels_copy`.
- Reset: width/height attribute set (HTML via reflected attr, Offscreen via slot), `reset_canvas_context_state`, backing-store reset, `reset_canvas_like_backing_store_for_dimension_assignment`.
- Retirement/adoption: `retire_canvas_resources_for_document` (adopts resolved by owner document).

**State vs geometry**
- State lives on V8 private slots (helpers.rs declaration + lineDash slot).
- Path geometry + transform lives in `Canvas2dPathState` (weak-keyed per-context store, `state.rs`).

---

## 5. Existing conformance/stub gaps (honest inventory)

These are **not** evidence of completed API support and are declared (per proposal
§8) either as out-of-scope API expansion or as correctness gaps to resolve in-line:
- `isPointInPath`/`isPointInStroke` always return `false` (STUB).
- `createLinearGradient`/`CanvasGradient.addColorStop` validate but never render gradients; `fillStyle`/`strokeStyle` are color-only strings (STUB-ish; gradient rendering is unrelated API expansion).
- `convertToBlob` returns an empty blob (STUB).
- `drawImage` supports the 3/5/9-arg forms via `DrawImageBlit`; HTMLVideoElement/CanvasImageSource breadth is limited.
- Text is `font8x8` monochrome glyphs only (functional, not a real font engine); this is the supported text path today and must be captured through the native recorder, not expanded.
- No `getTransform`, `reset`, `setLineDash` canonicalization quirks are necessarily complete; correctness is defined by existing JS regressions, not by parity claims.
- `globalCompositeOperation` is validated/canonicalized but only `source-over` semantics are actually composited.

These gaps do not block the architecture; per the proposal, only gaps that prevent
recording/ownership/readback/lifecycle correctness must be resolved in-project.

---

## 6. Reproducible baseline

Fixture + runner live under `moli-benchmark/fixtures/canvas/` (see
`moli-benchmark/fixtures/canvas/README.md` for how to run and reproduce).

Method: drive the Moli binary over CDP, load an HTML fixture for each workload
(256/1024/2048 canvas sizes × 100/1000 ops over path-fill, rect, image, text,
draw/clear/write, readback-after-every-draw, repeated clean getImageData, and
canvas-to-canvas/self-draw), measure wall time in JS around a recorded event loop,
and report per-phase timings. A Rust-side native cost model benchmark
(`moli-canvas/tests/baseline_cost.rs`, native, no V8) reports the full-plane copy
and full-raster counts for the current design to make the structural cost visible
independently of wall-clock noise.

Raw results, machine info, build mode, and revision are recorded in
`moli-benchmark/fixtures/canvas/README.md` and `moli-benchmark/fixtures/canvas/results/`.

### Captured native baseline (this machine; branch `canvas_2D_moli` @ `4d6e0373`, debug build)

The native cost model (`moli-canvas/tests/baseline_cost.rs`, V8-free) reproduces
the current design's per-draw full-plane copy + format-conversion work. Its
**arithmetic byte-cost evidence** (asserted, instant) shows the cost is linear in
canvas area, not paint size, because every ordinary draw copies the whole plane:

| canvas | ops | bytes/plane | full copies/draw | bytes copied (total) |
|---|---|---|---|---|
| 256² | 100 | 262,144 | 2 | 52,428,800 |
| 1024² | 1000 | 4,194,304 | 2 | 8,388,608,000 |
| 2048² | 1000 | 16,777,216 | 2 | 33,554,432,000 |

Moving 33.5 GB to draw 1,000 small shapes on a 2048² canvas is the structural
problem this project removes (path fills additionally allocate a fresh full-canvas
raster to composite). A reduced timing matrix is kept in the test so the check
suite stays fast; see `moli-benchmark/fixtures/canvas/results/README.md`.

### Baseline commands (run before any migration code)

```sh
# correctness regression baseline
cargo nextest run -p moli-renderer-v8 --lib canvas_paths canvas_arguments --no-fail-fast

# native cost model (no V8)
cargo test -p moli-canvas --test baseline_cost -- --nocapture
```

### Environment note

`moli-canvas` native tests pass (22/22) here. The `moli-renderer-v8` JS
regressions and the CDP wall-clock benchmark could **not** be executed in the M0
capture environment: rebuilding `aws-lc-sys` (pulled by the renderer test graph)
requires libclang/bindgen, which is not installed. The once-built `target/debug/moli`
predates this session and does not cover the fresh test build. The workload JS is
validated for correctness (all 9 fixtures produce well-formed `__canvasResult`
under a Node DOM stub), but wall-clock JS numbers must be captured at M6 on a
machine with libclang and a built `moli serve`.


---

## 7. Decisions carried forward

Recorded from the M0 discussion (see proposal §6.2 and the session decisions):

- **Internal surface format: premultiplied RGBA8** in the target core, matching
  Vello output. Conversion to straight alpha happens only at observation/export/
  publication boundaries (getImageData, toDataURL, page snapshots via
  `RgbaImage`, ImageData writes, canvas-as-source capture).
- **Backend: `moli-canvas` gains direct `anyrender` + `anyrender_vello_cpu`
  dependencies** (same pinned rev `18fd67d…` moli-paint uses) with its own
  `backend/vello_cpu.rs`. `moli-canvas` stays free of V8/DOM/layout/moli-paint.
- **Backend reuse**: one `VelloCpuImageRenderer` per canvas, sized to the surface,
  rendering into a caller-owned buffer (`render(&mut scene, &mut [u8])`), reused
  across flushes. Whether `RenderContext::reset()` retains the large fine-stage
  buffers is verified empirically in M2 (dedicated micro-benchmark).
- `moli_image::RgbaImage` is the published page-visible snapshot unit (straight
  alpha), already the type `CanvasResourceStore` stores and page painting consumes.

### M2 status (surface/backend contract implemented)

M2 delivered the independently tested canvas surface and reusable backend in
`moli-canvas`:

- `surface.rs::CanvasSurface` is the single authoritative writable pixel store:
  **premultiplied RGBA8** internally, with lazy materialization, a reusable
  backend, an immutable cached straight-alpha snapshot, region readback, and
  reset/resize. Deterministic `flush_count`/`snapshot_count` counters prove
  scheduling properties (N draws share the surface; a clean observation does
  zero additional conversion).
- `backend/vello_cpu.rs::VelloCpuBackend` reuses one `VelloCpuScenePainter` per
  canvas and renders into the caller-owned persistent buffer with
  `CompositeMode::SrcOver` (incremental source-over batches preserving prior
  content) or `CompositeMode::Replace` (destructive clear/overwrite). This
  uses `VelloCpuScenePainter`'s public `render_ctx`/`resources` rather than
  `VelloCpuImageRenderer`, whose `render` hardcodes `Replace`. Whether
  `RenderContext::reset()` retains the large fine-stage buffers remains an
  empirical M2 detail noted for the reuse benchmark.
- Pixel-format contract: premultiplied internal (Vello-native); `unpremultiply`
  happens only at snapshot/readback/export boundaries; transparent pixels
  normalize to transparent black so repeated conversion is stable.
- Native tests (`tests/surface_api.rs`, no V8/Document) cover repeated
  source-over rendering, snapshot isolation and clean repeated reads, clear
  ordering, reset/resize (same/different/zero size), region readback clipping
  and independence, low-alpha round-trip, oversized/invalid failure without
  mutation, and deterministic scheduling counters.

---

## 8. Checklist for final review (routed against this inventory)

Every functional 2D route above must, at M6, be accounted for by the final
architecture per the proposal's §5 table. The numbering above is the audit key:
- [ ] All **D** routes route through the native ordered recorder (M4).
- [ ] **W** boundaries (`putImageData`) are ordered native pixel writes.
- [ ] **R** routes (`getImageData`, exports, source-canvas, page painting, screencast) read a single authoritative surface/snapshot after flush (M5).
- [ ] **S** geometry/state operations update native state/path without rasterizing or flushing (M1/M4).
- [ ] **RST** dimension assignment/reset preserves the required reset semantics incl. same-size (M4).
- [ ] **STUB** items are either removed or honestly declared out-of-scope.
- [ ] The dual-plane backing store + full-frame raster path (`with_canvas_like_pixels_mut` for draws, `rasterize_canvas_fragment`) is removed (M6).
