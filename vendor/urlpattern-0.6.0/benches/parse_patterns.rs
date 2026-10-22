use criterion::{Criterion, black_box, criterion_group, criterion_main};
use urlpattern::quirks::{self, EcmaRegexp};

fn bench_parse_shipping_groups_summary(c: &mut Criterion) {
  c.bench_function("parse component-ShippingGroupsSummary.*.js", |b| {
    b.iter(|| {
      let input = quirks::process_construct_pattern_input(
        black_box(quirks::StringOrInit::String(
          "component-ShippingGroupsSummary.*.js".into(),
        )),
        black_box(Some("https://example.test/web/")),
      );
      quirks::parse_pattern::<EcmaRegexp>(
        input.unwrap(),
        urlpattern::UrlPatternOptions::default(),
      )
      .unwrap();
    })
  });
}

criterion_group!(benches, bench_parse_shipping_groups_summary);
criterion_main!(benches);
