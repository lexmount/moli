use super::*;

// A `container-type: inline-size` box applies inline-size containment: its
// inline extent must not depend on its contents. Previously a content-sized
// container (a row flex item with `flex-basis: auto`) resolved `cqw`-based
// font sizes against its own content-derived width and inflated past its flex
// parent. See https://github.com/lexmount/moli issue #1288.
#[test]
fn inline_size_container_does_not_inflate_from_cqw_font_sizes() {
    let mut vm = new_parsed_test_vm(
        "https://container-size-runaway.test/",
        r#"<!doctype html><style>
          :root { --fluid-unit: 100cqw; }
          body { margin: 0; }
          #feed { display: flex; width: 400px; }
          .teaser {
            display: flex;
            container-type: inline-size;
            font-size: calc(var(--fluid-unit) / 50);
          }
          .teaser span { white-space: nowrap; }
        </style>
        <section id="feed">
          <article class="teaser"><span>XXXXX XXXXX XXXXX XXXXX XXXXX XXXXX XXXXX</span></article>
        </section>"#,
    );
    refresh_layout_for_test(&mut vm);

    let result = vm
        .eval(
            r#"
        (() => {
          const feed = document.getElementById('feed');
          const teaser = feed.querySelector('.teaser');
          return JSON.stringify({
            feed: Math.round(feed.getBoundingClientRect().width),
            teaser: Math.round(teaser.getBoundingClientRect().width)
          });
        })()
      "#,
        )
        .expect("container query runaway probe should evaluate");

    let value: serde_json::Value = serde_json::from_str(&result).expect("probe JSON");
    let feed = value["feed"].as_f64().unwrap();
    let teaser = value["teaser"].as_f64().unwrap();

    assert_eq!(feed, 400.0, "feed width is authored");
    assert!(
        teaser <= feed,
        "inline-size container teaser ({teaser}) must not exceed its flex parent ({feed})"
    );
}
