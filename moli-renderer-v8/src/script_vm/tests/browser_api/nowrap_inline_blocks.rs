use super::*;

// `white-space: nowrap` suppresses every soft wrap opportunity, including the
// boundaries between adjacent inline-block boxes. Previously an IFC whose
// content began with inline blocks and carried no styled text never seeded
// Parley's line wrap mode, so the boxes wrapped onto extra lines.
// See https://github.com/lexmount/moli issue #804.
#[test]
fn nowrap_keeps_inline_blocks_on_one_line() {
    let mut vm = new_parsed_test_vm(
        "https://nowrap-inline-blocks.test/",
        r#"<!doctype html><style>
          #c { white-space: nowrap; width: 250px; }
          #c span { display: inline-block; width: 100px; height: 20px; background: #ddd; }
        </style>
        <body><div id="c"><span>1</span><span>2</span><span>3</span><span>4</span><span>5</span></div></body>"#,
    );
    refresh_layout_for_test(&mut vm);

    let result = vm
        .eval(
            r#"
        (() => {
          const c = document.getElementById('c');
          const spans = [...c.querySelectorAll('span')];
          return JSON.stringify({
            tops: spans.map(s => Math.round(s.getBoundingClientRect().top)),
            scrollWidth: Math.round(c.scrollWidth)
          });
        })()
      "#,
        )
        .expect("nowrap inline-block probe should evaluate");

    let value: serde_json::Value = serde_json::from_str(&result).expect("probe JSON");
    let tops: Vec<i64> = value["tops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    let scroll_width = value["scrollWidth"].as_i64().unwrap();

    assert_eq!(
        tops,
        vec![8, 8, 8, 8, 8],
        "all inline blocks must stay on a single line under white-space: nowrap"
    );
    assert_eq!(scroll_width, 500, "the five 100px boxes occupy 500px");
}

// The defect was specific to wrap points between inline blocks; a pure-text
// nowrap control already wrapped correctly and must remain intact. Compare the
// nowrap line height against the wrapping baseline: nowrap must use fewer lines.
#[test]
fn nowrap_pure_text_line_is_unaffected() {
    let mut vm = new_parsed_test_vm(
        "https://nowrap-text.test/",
        r#"<!doctype html><style>
          .c { width: 250px; }
        </style>
        <body>
          <div class="c" style="white-space: nowrap" id="nowrap">aaaaaaaaaa bbbbbbbbbb cccccccccc dddddddddd eeeeeeeeee</div>
          <div class="c" id="wrap">aaaaaaaaaa bbbbbbbbbb cccccccccc dddddddddd eeeeeeeeee</div>
        </body>"#,
    );
    refresh_layout_for_test(&mut vm);

    let result = vm
        .eval(
            r#"
        (() => {
          const nowrap = document.getElementById('nowrap');
          const wrap = document.getElementById('wrap');
          return JSON.stringify({
            nowrapHeight: Math.round(nowrap.getBoundingClientRect().height),
            wrapHeight: Math.round(wrap.getBoundingClientRect().height)
          });
        })()
      "#,
        )
        .expect("nowrap text probe should evaluate");

    let value: serde_json::Value = serde_json::from_str(&result).expect("probe JSON");
    let nowrap_height = value["nowrapHeight"].as_i64().unwrap();
    let wrap_height = value["wrapHeight"].as_i64().unwrap();

    assert!(
        nowrap_height < wrap_height,
        "nowrap text ({nowrap_height}) must occupy fewer lines than wrapping text ({wrap_height})"
    );
}
