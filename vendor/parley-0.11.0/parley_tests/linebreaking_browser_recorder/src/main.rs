// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! In-browser data-collection harness for Parley/Chrome line-breaking parity.
//!
//! For each test case (see [`parley_linebreaking_cases`]), we find where the browser first breaks,
//! given a random width. We then find and record the minimum width which also causes the same break.
//! A future extension will be to evaluate with a more reasonable bound across multiple lines of text;
//! this test is designed to catch the edge cases for that test.
//!
//! Build/run with [Trunk](https://trunkrs.dev):
//!
//! ```sh
//! rustup target add wasm32-unknown-unknown
//! cargo install --locked trunk
//! # If needed:
//! cd parley_tests/linebreaking_browser_recorder
//! trunk serve
//! ```
//!
//! Trunk runs the driver in the working directory, so must be run in this
//! package's directory.
//! You can then open the application in Chrome (or another Chromium based browser)
//! to get the current data.
//!
//! The data is persisted manually, by copying it into the relevant files, using the relevant
//! `Copy <font> Data` buttons. These should go into the appropriately named files in
//! `parley_tests/linebreaking_browser_recorder/data`

use js_sys::Uint8Array;
use parley_linebreaking_cases::{Case, FONTS, PROBE_SUBPIXELS, SUBPIXELS_PER_PX};
use std::fmt::Write as _;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Document, Element, FontFace, HtmlElement, Node, Range, Window};

/// A fixed line height gives stable, well-separated per-line `y` positions,
/// which is what [`first_line_break`] reads to tell lines apart.
const LINE_HEIGHT: f64 = 50.0;

/// Number of seeds to collect by default (overridable with `?count=N`).
const DEFAULT_SEED_COUNT: u64 = 1024;

struct Record {
    seed: u64,
    /// The tightest width that still keeps the first line intact, as an
    /// integer number of subpixels ([`SUBPIXELS_PER_PX`]).
    ///
    /// A value of `0` is a sentinel meaning the first line had no interior break
    /// opportunity. In that case, we just validate that Parley chooses to break
    /// at the same character. See [`PROBE_SUBPIXELS`] for more details.
    tightened_width_subpixels: i64,
    /// Characters on the first line. We use this to confirm
    /// that the same place in the line was used by Parley to break.
    first_line_chars: usize,
}

fn main() {
    console_error_panic_hook::set_once();
    wasm_bindgen_futures::spawn_local(async {
        if let Err(err) = run().await {
            web_sys::console::error_1(&err);
        }
    });
}

async fn run() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;

    set_status(&document, "Loading fonts…")?;
    load_fonts(&window).await?;

    let measurer = make_measurer(&document)?;
    let results = document.get_element_by_id("results").ok_or("no #results")?;

    let version = chrome_version(&window);

    for font in FONTS {
        measurer.style().set_property("font-family", font.family)?;

        let mut records = Vec::new();
        for seed in 0..DEFAULT_SEED_COUNT {
            if seed % 512 == 0 {
                set_status(
                    &document,
                    &format!(
                        "Collecting {} - {}/{}",
                        font.family, seed, DEFAULT_SEED_COUNT
                    ),
                )?;
                yield_now().await;
            }
            records.push(collect(&Case::from_seed(seed), &measurer, &document));
        }

        let csv = output_csv(font.family, &version, &records);
        setup_copy_button(&document, &results, font.family, csv)?;
    }

    measurer.remove();
    set_status(
        &document,
        &format!(
            "Done — {} font(s); use the per-font buttons to get the data. This can then be copied into the relevant data files.",
            FONTS.len()
        ),
    )?;
    Ok(())
}

/// Collect the first-line break decision for a single case.
fn collect(case: &Case, measurer: &HtmlElement, document: &Document) -> Record {
    let text = &case.text;
    // We floor here arbitrarily; we just need some mapping from the initial width to the subpixel width.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "Reasonable initial width values and floor means this will never truncate.."
    )]
    let initial_width_subpixels = (f64::from(case.initial_width) * SUBPIXELS_PER_PX).floor() as i64;

    measurer
        .style()
        .set_property("font-size", &format!("{}px", case.font_size))
        .unwrap();

    measurer.set_text_content(Some(text));
    let text_node = measurer.first_child().unwrap();
    assert_eq!(
        text_node.node_type(),
        Node::TEXT_NODE,
        "Measurer child node must be a text node for the Range API to work as we expect."
    );
    let range = document.create_range().unwrap();
    assert!(
        text.is_ascii(),
        "This code is not written to be robust against non-ASCII text, due to the DOM's use of UTF-16."
    );
    let len = text.len();

    // The index in the text at which the first line breaks, at the initial width.
    let break_at = first_line_break(measurer, &range, &text_node, len, initial_width_subpixels);
    // The index of the first break opportunity in the line.
    let first_break = first_line_break(measurer, &range, &text_node, len, PROBE_SUBPIXELS);

    debug_assert!(
        first_break <= break_at,
        "Making text narrower can't move break to later in the text."
    );
    let tightened_width_subpixels = if first_break == break_at {
        0
    } else {
        // Shrink the width to the minimum that still breaks here.
        tighten_width_subpixels(
            measurer,
            &range,
            &text_node,
            len,
            break_at,
            initial_width_subpixels,
        )
    };

    Record {
        seed: case.seed,
        tightened_width_subpixels,
        first_line_chars: break_at,
    }
}

/// Use `range` to find the y-position of the character at the given UTF-16 `index` within the `node`.
///
/// `node` must be a text node.
#[expect(
    clippy::cast_possible_truncation,
    reason = "text lengths are tiny, far inside u32"
)]
fn char_top(range: &Range, node: &Node, index: usize) -> f64 {
    range.set_start(node, index as u32).unwrap();
    range.set_end(node, index as u32 + 1).unwrap();
    range.get_bounding_client_rect().top()
}

/// The length of the text on the first line. A return value of `len` means that all the text was on the first line.
fn first_line_break(
    measurer: &HtmlElement,
    range: &Range,
    node: &Node,
    len: usize,
    width_subpixels: i64,
) -> usize {
    measurer
        .style()
        .set_property("width", &format!("{}px", width_px(width_subpixels)))
        .unwrap();

    let first_top = char_top(range, node, 0);
    if char_top(range, node, len - 1) <= first_top {
        return len;
    }
    // Invariant: the char at `lo` is on line one, the char at `hi` is later.
    let mut lo = 0;
    let mut hi = len - 1;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if char_top(range, node, mid) > first_top {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// Find the smallest width, in integer subpixels, where the first line still breaks
/// at `target_break`.
///
/// We know that there will be such a value because shrinking the width won't allow more
/// content onto the line with the greedy algorithm we're trying to match.
fn tighten_width_subpixels(
    measurer: &HtmlElement,
    range: &Range,
    node: &Node,
    len: usize,
    target_break: usize,
    hi_subpixels: i64,
) -> i64 {
    // Invariant: the break is before `target_break` at `lo`, at it at `hi`.
    let mut lo = PROBE_SUBPIXELS;
    let mut hi = hi_subpixels;
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        let preserved = first_line_break(measurer, range, node, len, mid) >= target_break;
        if preserved {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// Convert a width in integer subpixels back to CSS pixels.
const fn width_px(width_subpixels: i64) -> f64 {
    width_subpixels as f64 / SUBPIXELS_PER_PX
}

/// Register and load every supported font, awaiting their readiness.
async fn load_fonts(window: &Window) -> Result<(), JsValue> {
    let fonts = window.document().ok_or("no document")?.fonts();
    for font in FONTS {
        let bytes = Uint8Array::from(font.bytes);
        let face = FontFace::new_with_array_buffer(font.family, &bytes.buffer())?;
        // We could parallelise this, but with only two fonts it doesn't have much advantage.
        let loaded = JsFuture::from(face.load()?).await?;
        let face: FontFace = loaded.dyn_into()?;
        fonts.add(&face)?;
    }
    Ok(())
}

/// We measure text using an off-screen element.
fn make_measurer(document: &Document) -> Result<HtmlElement, JsValue> {
    let el: HtmlElement = document.create_element("div")?.dyn_into()?;
    let style = el.style();
    // Avoid rendering the element, to speed up processing.
    style.set_property("position", "absolute")?;
    style.set_property("left", "-99999px")?;
    style.set_property("top", "0")?;
    style.set_property("visibility", "hidden")?;
    // Ensure that our the values we read are only due to the text.
    style.set_property("margin", "0")?;
    style.set_property("padding", "0")?;
    style.set_property("border", "0")?;
    // Match Parley's line breaking properties.
    style.set_property("white-space", "normal")?;
    style.set_property("word-break", "normal")?;
    style.set_property("overflow-wrap", "normal")?;
    style.set_property("line-height", &format!("{LINE_HEIGHT}px"))?;
    document.body().ok_or("no body")?.append_child(&el)?;
    Ok(el)
}

fn set_status(document: &Document, message: &str) -> Result<(), JsValue> {
    if let Some(status) = document.get_element_by_id("status") {
        status.set_text_content(Some(message));
    }
    Ok(())
}

/// Serialise the collected data as CSV.
///
/// We also store some metadata as frontmatter in comments indicated by '#' - that isn't standard, but
/// we make sure that our parser ignores it.
fn output_csv(font_family: &str, chrome_version: &str, records: &[Record]) -> String {
    let mut out = format!(
        "# font_family: {font_family}
# chrome_version: {chrome_version}
seed,width_subpixels,first_line_chars\n"
    );
    for r in records {
        writeln!(
            out,
            "{},{},{}",
            r.seed, r.tightened_width_subpixels, r.first_line_chars
        )
        .unwrap();
    }
    out
}

/// The Chrome/Chromium version from the user-agent string (e.g. `"138.0.0.0"`),
/// falling back to the full user-agent if it is not Chrome-shaped.
fn chrome_version(window: &Window) -> String {
    let user_agent = window.navigator().user_agent().unwrap_or_default();
    match user_agent.split_once("Chrome/") {
        Some((_, rest)) => rest.split(' ').next().unwrap_or("").to_owned(),
        None => user_agent,
    }
}

/// Ceeate the button which copies the data for a given font to the clipboard.
fn setup_copy_button(
    document: &Document,
    results: &Element,
    family: &str,
    csv: String,
) -> Result<(), JsValue> {
    let button: HtmlElement = document.create_element("button")?.dyn_into()?;
    button.set_text_content(Some(&format!("Copy {family} Data")));
    results.append_child(&button)?;

    let label = format!("Copied {family} ✓");
    let button_for_handler = button.clone();
    let on_click = Closure::<dyn FnMut()>::new(move || {
        if let Some(window) = web_sys::window() {
            // The returned promise is fire-and-forget; we don't await it.
            let _ = window.navigator().clipboard().write_text(&csv);
        }
        button_for_handler.set_text_content(Some(&label));
    });
    button.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
    // Keep the closure alive for the lifetime of the page.
    on_click.forget();
    Ok(())
}

/// Yield to other tasks in the browser. In particular, this allows painting/redrawing to happen, so that progress is visible.
/// Adapted from <https://github.com/wasm-bindgen/wasm-bindgen/discussions/3476#discussion-5283084>
async fn yield_now() {
    let mut cb = |resolve: js_sys::Function, _reject: js_sys::Function| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0)
            .expect("Failed to call set_timeout");
    };
    let p = js_sys::Promise::new(&mut cb);
    JsFuture::from(p).await.unwrap();
}
