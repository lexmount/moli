//! Opt-in, release-mode font-loading measurements. No wall-clock assertions.
//!
//! cargo test -p moli-layout --release --lib text::benchmarks::font_loading \
//!     -- --ignored --nocapture
//! MOLI_FONT_BENCH_EXTRA may name one additional local font (read before timing).

use std::{hint::black_box, time::Instant};

use super::*;

const SAMPLES: usize = 7;

fn median_ms(mut sample: impl FnMut() -> f64) -> f64 {
    // Warm code/data separately from the measured samples.
    black_box(sample());
    let mut samples: Vec<_> = (0..SAMPLES).map(|_| sample()).collect();
    samples.sort_by(f64::total_cmp);
    samples[SAMPLES / 2]
}

fn batch_registration(bytes: &[u8], count: usize, initialized: bool, unchanged: bool) -> f64 {
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    if initialized {
        black_box(services.parley_mut());
    }
    let registrations: Vec<_> = (0..count)
        .map(|index| {
            let index = if unchanged { 0 } else { index };
            WebFontRegistration::new(
                format!("slot-{index:04}"),
                WebFontFace::new(format!("Family {index:04}")),
                bytes.to_vec(),
            )
        })
        .collect();
    if unchanged {
        services
            .register_web_font(registrations[0].clone())
            .unwrap();
    }
    // Payload copies, context initialization and destruction are not timed.
    let start = Instant::now();
    for registration in registrations {
        black_box(services.register_web_font(black_box(registration)).unwrap());
    }
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(services.web_font_count(), if unchanged { 1 } else { count });
    black_box(&services);
    elapsed
}

#[test]
#[ignore = "opt-in release microbenchmark; not a correctness or CI timing gate"]
fn font_loading() {
    let mut fonts = vec![
        (
            "ahem-ttf".to_owned(),
            include_bytes!("../../tests/fixtures/moli-ahem.ttf").to_vec(),
        ),
        (
            "ahem-woff".to_owned(),
            include_bytes!("../../tests/fixtures/moli-ahem.woff").to_vec(),
        ),
        (
            "ahem-woff2".to_owned(),
            include_bytes!("../../tests/fixtures/moli-ahem.woff2").to_vec(),
        ),
    ];
    if let Some(path) = std::env::var_os("MOLI_FONT_BENCH_EXTRA") {
        fonts.push((
            path.to_string_lossy().into_owned(),
            std::fs::read(path).unwrap(),
        ));
    }
    println!("font,bytes,operation,count,median_ms");
    for (name, bytes) in fonts {
        let iterations = if bytes.len() > 1_000_000 { 8 } else { 128 };
        for (operation, validate) in [("decode", false), ("validate", true)] {
            let elapsed = median_ms(|| {
                let start = Instant::now();
                for _ in 0..iterations {
                    if validate {
                        validate_web_font_bytes(black_box(&bytes)).unwrap();
                    } else {
                        black_box(decode_web_font_bytes(black_box(&bytes)).unwrap());
                    }
                }
                start.elapsed().as_secs_f64() * 1000.0 / iterations as f64
            });
            println!("{name},{},{operation},1,{elapsed:.6}", bytes.len());
        }
        let counts: &[usize] = if bytes.len() > 1_000_000 {
            &[1, 4, 8]
        } else {
            &[1, 16, 64, 128]
        };
        for &count in counts {
            for (operation, initialized, unchanged) in [
                ("register-lazy", false, false),
                ("register-initialized", true, false),
                ("register-unchanged", true, true),
            ] {
                let elapsed =
                    median_ms(|| batch_registration(&bytes, count, initialized, unchanged));
                println!("{name},{},{operation},{count},{elapsed:.6}", bytes.len());
            }
        }
    }
}
