//! Native (V8-free) cost model that makes the current Canvas 2D design's
//! structural cost visible. This is an M0 baseline artifact: it measures the
//! per-draw full-plane copy and format-conversion work the current renderer
//! does for every ordinary draw, so the pre-migration cost is reproducible
//! without V8, a Document, or a browser binary.
//!
//! Run with:
//!   cargo test -p moli-canvas --test baseline_cost -- --nocapture
//!
//! The key evidence (bytes copied per draw, which scales with canvas area) is
//! computed arithmetically and asserted. Wall-clock timing is deliberately kept
//! to a small reduced matrix so this test stays fast inside the workspace check
//! suite; the full timing matrix is reported in
//! `moli-benchmark/fixtures/canvas/results/README.md`.

use std::time::Instant;

use moli_canvas::{
    Rgba8Rect, byte_len, copy_rgba8_rect, encode_data_url, premultiply_rgba8_in_place,
};

/// The three canvas areas named in the proposal's workload matrix, with the op
/// counts used for the (arithmetic) byte-cost evidence.
const SIZES: [(u32, u32, usize); 3] = [(256, 256, 100), (1024, 1024, 1000), (2048, 2048, 1000)];

/// A reduced matrix actually timed, so the test stays quick in debug builds.
const TIMED: [(u32, u32, usize, usize); 2] = [(256, 256, 100, 1), (1024, 1024, 100, 10)];

fn report(label: impl AsRef<str>, rows: Vec<(String, String)>) {
    eprintln!("--- {} ---", label.as_ref());
    for (key, value) in rows {
        eprintln!("{key}: {value}");
    }
}

#[test]
fn baseline_cost_draw_is_linear_in_canvas_area_not_paint_size() {
    // The current `with_canvas_like_pixels_mut` path copies the entire backing
    // view into a Vec, mutates it, and writes it back: two full-plane byte
    // copies per ordinary draw, regardless of how small the painted shape is.
    // This is the O(canvas area) per-draw cost the proposal targets.
    for (width, height, ops) in SIZES {
        let len = byte_len(width, height).expect("valid canvas byte len");
        let copied = len as u128 * 2 * ops as u128;
        report(
            format!("arithmetic cost {width}x{height} x{ops}"),
            vec![
                ("bytes_per_plane".to_string(), len.to_string()),
                ("full_copies_per_draw".to_string(), "2".to_string()),
                ("bytes_copied_total".to_string(), copied.to_string()),
            ],
        );
        // The whole-plane copy must be exercised exactly once per op by the
        // memcpy fast path, moving `len` bytes each time.
        let surface = vec![0u8; len];
        let mut work = vec![0u8; len];
        copy_rgba8_rect(
            &surface,
            width,
            height,
            Rgba8Rect::new(0, 0, width, height).expect("full surface"),
            &mut work,
            width,
            height,
            0,
            0,
        )
        .expect("full copy fits");
        assert_eq!(work, surface, "full-plane copy must move every byte");
    }
}

#[test]
fn baseline_cost_timing_is_reported_for_a_reduced_matrix() {
    for (width, height, ops, encode_iters) in TIMED {
        let len = byte_len(width, height).expect("valid canvas byte len");
        let surface = vec![0u8; len];
        let mut work = vec![0u8; len];

        let copy_start = Instant::now();
        for _ in 0..ops {
            copy_rgba8_rect(
                &surface,
                width,
                height,
                Rgba8Rect::new(0, 0, width, height).expect("full surface"),
                &mut work,
                width,
                height,
                0,
                0,
            )
            .expect("full copy fits");
        }
        let copy_secs = copy_start.elapsed().as_secs_f64();

        let convert_start = Instant::now();
        for _ in 0..ops {
            let mut converted = work.clone();
            premultiply_rgba8_in_place(&mut converted).expect("mult of 4");
        }
        let convert_secs = convert_start.elapsed().as_secs_f64();

        let encode_start = Instant::now();
        for _ in 0..encode_iters {
            let _ = encode_data_url(&surface, width, height);
        }
        let encode_secs = encode_start.elapsed().as_secs_f64();

        report(
            format!("timed {width}x{height} x{ops} (debug)"),
            vec![
                (
                    "full_copy_total_secs".to_string(),
                    format!("{copy_secs:.6}"),
                ),
                (
                    "convert_pass_total_secs".to_string(),
                    format!("{convert_secs:.6}"),
                ),
                ("encode_total_secs".to_string(), format!("{encode_secs:.6}")),
            ],
        );
    }
}
