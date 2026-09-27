use super::*;

#[test]
fn document_point_queries_use_real_paint_order_geometry() {
    let mut vm = new_rendered_test_vm(
        "https://document-point-query.test/path/index.html",
        r#"<html><body>
            <div id="target" style="width:100px;height:50px">target</div>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => JSON.stringify({
  element: document.elementFromPoint(10, 10)?.localName ?? null,
  elements: document.elementsFromPoint(10, 10).map(element => element.localName)
}))()
"#,
        )
        .expect("document point query probe should evaluate");

    assert_eq!(
        result,
        r#"{"element":"div","elements":["div","body","html"]}"#
    );
}

#[test]
fn style_source_sync_consumes_prepared_sources_and_preserves_cssom() {
    let mut vm = new_parsed_test_vm(
        "https://style-source-lifecycle.test/",
        "<!doctype html><body></body>",
    );
    vm.eval(
        r#"
document.open();
document.write('<!doctype html><head><style id=sheet>body { color: red; }');
globalThis.pendingStyle = document.getElementById('sheet');
"#,
    )
    .unwrap();

    vm.sync_live_document_style_sources();
    assert_eq!(
        vm.eval("JSON.stringify([pendingStyle.sheet === null, document.styleSheets.length])")
            .unwrap(),
        "[true,0]",
        "rendering synchronization must not create a source for an unfinished style",
    );

    vm.eval(
        r#"
document.write('</style></head><body>');
globalThis.preparedSheet = pendingStyle.sheet;
preparedSheet.insertRule('.added { color: blue; }', preparedSheet.cssRules.length);
"#,
    )
    .unwrap();
    vm.sync_live_document_style_sources();
    assert_eq!(
        vm.eval(
            "JSON.stringify([pendingStyle.sheet === preparedSheet, preparedSheet.cssRules.length])"
        )
        .unwrap(),
        "[true,2]",
        "synchronization must preserve the prepared stylesheet and CSSOM edits",
    );
    assert_eq!(
        vm.eval(
            r#"
pendingStyle.textContent = 'body { color: green; }';
document.close();
JSON.stringify([pendingStyle.sheet !== preparedSheet,
  pendingStyle.sheet.cssRules.length, getComputedStyle(document.body).color]);
"#,
        )
        .unwrap(),
        r#"[true,1,"rgb(0, 128, 0)"]"#,
        "a real DOM content change must still replace the stylesheet",
    );
}

#[test]
fn parser_style_waits_for_complete_source_in_main_and_child_documents() {
    let mut vm = new_parsed_test_vm(
        "https://parser-style-completion.test/",
        "<!doctype html><body></body>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  function exercise(w) {
    const d = w.document;
    d.open();
    d.write('<!doctype html><head><style id=sheet>\n');
    const style = d.getElementById('sheet');
    const pending = () => [style.sheet === null, d.styleSheets.length];
    const before = [pending()];
    d.write('body { color: rgb(1, 2, 3); }\n');
    before.push(pending());
    d.write('.second { color: green; }\n');
    style.type = 'text/css';
    before.push(pending());
    const clone = d.head.appendChild(style.cloneNode(true));
    const cloneRules = clone.sheet.cssRules.length;
    clone.remove();
    d.write('</style></head><body>');
    const sheet = style.sheet;
    const complete = [sheet.cssRules.length, d.styleSheets.length,
      w.getComputedStyle(d.body).color];
    sheet.insertRule('.kept { color: blue; }', sheet.cssRules.length);
    d.write('<p>unrelated parser work</p>');
    const preserved = sheet === style.sheet && sheet.cssRules.length === 3;
    style.firstChild.appendData('.third { color: black; }');
    const changed = Array.from(style.sheet.cssRules, r => r.selectorText);
    style.textContent = 'body { color: rgb(4, 5, 6); }';
    const dynamicColor = w.getComputedStyle(d.body).color;
    d.close();
    return {before, cloneRules, complete, preserved, changed, dynamicColor};
  }
  const main = exercise(window);
  const frame = document.body.appendChild(document.createElement('iframe'));
  const child = exercise(frame.contentWindow);
  return JSON.stringify([main, child]);
})()
"#,
        )
        .expect("parser style completion should preserve CSSOM and dynamic text updates");
    let expected = serde_json::json!({
        "before": [[true, 0], [true, 0], [true, 0]],
        "cloneRules": 2,
        "complete": [2, 1, "rgb(1, 2, 3)"],
        "preserved": true,
        "changed": ["body", ".second", ".third"],
        "dynamicColor": "rgb(4, 5, 6)"
    });
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([expected, expected]),
    );
}

#[test]
fn parser_style_completion_follows_the_creating_parser_and_preserves_abort() {
    let mut vm = new_parsed_test_vm(
        "https://parser-style-ownership.test/",
        "<!doctype html><body><div id=probe>probe</div></body>",
    );
    let result = vm.eval(r#"
(() => {
  const frame = document.body.appendChild(document.createElement('iframe'));
  const d = frame.contentDocument;
  d.open();
  d.write('<!doctype html><head><style>#probe { color: rgb(1, 2, 3) }');
  const moved = d.querySelector('style');
  document.head.appendChild(moved);
  const pending = moved.sheet === null;
  d.write('</style>');
  d.close();
  const complete = [!!moved.sheet, getComputedStyle(document.getElementById('probe')).color];
  moved.remove();

  d.open();
  d.write('<!doctype html><head><style>#probe { color: red }');
  const aborted = d.querySelector('style');
  d.open();
  d.write('<!doctype html><body>replacement</body>');
  d.close();
  document.head.appendChild(aborted);
  aborted.textContent = '#probe { color: blue }';
  const afterAbort = [aborted.sheet === null, getComputedStyle(document.getElementById('probe')).color];
  aborted.remove();
  frame.remove();
  return JSON.stringify({pending, complete, afterAbort});
})()
"#).expect("moving a style preserves parser ownership, while cancellation cannot complete it");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "pending": true,
            "complete": [true, "rgb(1, 2, 3)"],
            "afterAbort": [true, "rgb(0, 0, 0)"]
        }),
    );
}

#[test]
fn parser_style_finishes_at_eof_in_main_and_child_documents() {
    let mut vm = new_parsed_test_vm(
        "https://parser-style-eof.test/",
        "<!doctype html><body></body>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  function exercise(w) {
    const d = w.document;
    d.open();
    d.write('<!doctype html><body><style id=sheet>\nbody { color: rgb(7, 8, 9); }\n');
    const style = d.getElementById('sheet');
    const pending = style.sheet === null && d.styleSheets.length === 0;
    d.close();
    return [pending, style.sheet.cssRules.length, d.styleSheets.length,
      w.getComputedStyle(d.body).color];
  }
  const main = exercise(window);
  const frame = document.body.appendChild(document.createElement('iframe'));
  const child = exercise(frame.contentWindow);
  return JSON.stringify([main, child]);
})()
"#,
        )
        .expect("EOF should publish the complete unterminated style source");
    let expected = serde_json::json!([true, 1, 1, "rgb(7, 8, 9)"]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([expected, expected]),
    );
}

#[test]
fn parser_style_finishes_in_svg_and_detached_fragments() {
    let mut vm = new_parsed_test_vm(
        "https://parser-svg-style-completion.test/",
        "<!doctype html><body></body>",
    );
    let result = vm.eval(r#"
(() => {
  document.open();
  document.write('<!doctype html><body><svg><style id=sheet>\n');
  const style = document.getElementById('sheet');
  document.write('body { color: rgb(1, 2, 3); }\n');
  const pending = style.sheet === null && document.styleSheets.length === 0;
  document.write('</style><style id=empty /></svg>');
  const complete = [style.sheet.cssRules.length,
    document.getElementById('empty').sheet.cssRules.length];
  document.close();
  const fragment = document.createElement('div');
  fragment.innerHTML = '<style>body { color: rgb(4, 5, 6); }\n</style>';
  document.body.appendChild(fragment);
  const fragmentRules = fragment.firstChild.sheet.cssRules.length;
  const parsed = new DOMParser().parseFromString(
    '<html xmlns="http://www.w3.org/1999/xhtml"><head><style>body {color:red}</style></head></html>',
    'application/xhtml+xml');
  return JSON.stringify({pending, complete, fragmentRules,
    color:getComputedStyle(document.body).color,
    xmlRules:parsed.querySelector('style').sheet.cssRules.length});
})()
"#).expect("SVG, fragments and XML should all finish parser-created styles");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "pending": true, "complete": [1, 0], "fragmentRules": 1,
            "color": "rgb(4, 5, 6)", "xmlRules": 1
        }),
    );
}

#[test]
fn parser_coalesced_style_text_updates_main_and_child_hit_tests() {
    let mut vm = new_rendered_test_vm(
        "https://parser-style-hit-test.test/",
        "<!doctype html><body></body>",
    );
    let result = eval_with_layout_publications(
        &mut vm,
        r#"
(function* () {
  function* exercise(w) {
    const d = w.document;
    d.open();
    d.write('<!doctype html><style>\n');
    d.write('html, body {margin:0;padding:0} #target {width:100px;height:100px}');
    d.write('</style><body><div id=target></div>');
yield; // Publish this scene before reading its geometry.
    const rect = d.getElementById('target').getBoundingClientRect();
    const result = {rect:[rect.x,rect.y,rect.width,rect.height],
      hits:d.elementsFromPoint(1,1).map(e => e.id || e.localName)};
    d.close();
    return result;
  }
  const main = yield* exercise(window);
  const frame = document.body.appendChild(document.createElement('iframe'));
  const child = yield* exercise(frame.contentWindow);
  return JSON.stringify([main,child]);
})()
"#,
    )
    .expect("coalesced parser text must update its stylesheet before layout queries");
    let expected = serde_json::json!({"rect":[0,0,100,100], "hits":["target","body","html"]});
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([expected, expected]),
    );
}

#[test]
fn parser_coalesced_text_notifies_main_and_child_mutation_observers() {
    let mut vm = new_parsed_test_vm(
        "https://parser-text-observer.test/",
        "<!doctype html><body></body>",
    );
    let result = vm.eval(r#"
(() => {
  function exercise(w) {
    const d = w.document;
    d.open(); d.write('<!doctype html><body><p>a');
    const p = d.querySelector('p'), text = p.firstChild;
    const observer = new w.MutationObserver(() => {});
    observer.observe(p, {subtree:true,childList:true,characterData:true,characterDataOldValue:true});
    d.write('bc');
    const result = {sameText:p.firstChild === text, children:p.childNodes.length,
      records:observer.takeRecords().map(r => [r.type,r.target === text,r.oldValue,r.target.data])};
    observer.disconnect();
    d.write('</p>'); d.close();
    return result;
  }
  const main = exercise(window);
  const frame = document.body.appendChild(document.createElement('iframe'));
  return JSON.stringify([main,exercise(frame.contentWindow)]);
})()
"#).expect("parser text coalescing must report characterData without replacing the Text node");
    let expected = serde_json::json!({"sameText":true,"children":1,
        "records":[["characterData",true,"a","abc"]]});
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([expected, expected]),
    );
}

#[test]
fn point_queries_only_follow_explicit_viewport_publication() {
    use moli_layout::{
        GeometryProvider, LayoutPoint, LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch,
        LayoutViewport,
    };

    let mut vm = new_parsed_test_vm(
        "https://point-query-viewport.test/",
        "<html><body style='margin:0'><div style='width:300px;height:100px'></div></body></html>",
    );
    let before = vm.layout_pass_observability_for_test().1;
    let batch = LayoutQueryBatch::new(vec![
        LayoutQuery::HitTest {
            point: LayoutPoint::new(-1.0, -1.0),
            ignore_pointer_events_none: false,
        },
        LayoutQuery::HitTestAll {
            point: LayoutPoint::new(-1.0, -1.0),
            ignore_pointer_events_none: false,
        },
        LayoutQuery::HitTest {
            point: LayoutPoint::new(150.0, 20.0),
            ignore_pointer_events_none: false,
        },
        LayoutQuery::HitTestAll {
            point: LayoutPoint::new(150.0, 20.0),
            ignore_pointer_events_none: false,
        },
        LayoutQuery::CaretPosition {
            point: LayoutPoint::new(-1.0, -1.0),
        },
        LayoutQuery::CaretPosition {
            point: LayoutPoint::new(150.0, 20.0),
        },
    ]);
    for (width, expect_hit, expected_passes) in [
        (320, true, 1),
        (320, true, 1),
        (100, false, 2),
        (100, false, 2),
        (320, true, 3),
    ] {
        if vm.layout_pass_observability_for_test().1 < before + expected_passes {
            vm.screenshot_layout_snapshot(LayoutViewport::new(width, 200, 1.0))
                .unwrap()
                .unwrap();
        }
        let answers = GeometryProvider::answer(&mut *vm, &batch).expect("point query batch");
        assert_eq!(answers.answers[0], LayoutQueryAnswer::HitTest(None));
        assert_eq!(
            answers.answers[1],
            LayoutQueryAnswer::HitTestAll(Vec::new())
        );
        assert_eq!(
            matches!(answers.answers[2], LayoutQueryAnswer::HitTest(Some(_))),
            expect_hit
        );
        let LayoutQueryAnswer::HitTestAll(hits) = &answers.answers[3] else {
            panic!("expected hit list")
        };
        assert_eq!(!hits.is_empty(), expect_hit);
        assert_eq!(answers.answers[4], LayoutQueryAnswer::CaretPosition(None));
        let LayoutQueryAnswer::CaretPosition(position) = &answers.answers[5] else {
            panic!("expected caret position")
        };
        assert_eq!(position.is_some(), expect_hit);
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            before + expected_passes,
            "only explicit output publishes the changed viewport"
        );
    }
}

#[test]
fn point_queries_follow_explicit_output_when_viewport_expands() {
    use moli_layout::{
        GeometryProvider, LayoutPoint, LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch,
        LayoutViewport,
    };

    for (initial_size, expanded_size, point) in [
        ((100, 200), (320, 200), LayoutPoint::new(150.0, 20.0)),
        ((320, 100), (320, 200), LayoutPoint::new(20.0, 150.0)),
        ((320, 100), (100, 200), LayoutPoint::new(20.0, 150.0)),
    ] {
        let mut vm = new_parsed_test_vm(
            "https://point-query-viewport-expansion.test/",
            "<html><body style='margin:0'><div style='width:300px;height:300px'></div></body></html>",
        );
        let before = vm.layout_pass_observability_for_test().1;
        let batch = LayoutQueryBatch::new(vec![
            LayoutQuery::HitTest {
                point,
                ignore_pointer_events_none: false,
            },
            LayoutQuery::HitTestAll {
                point,
                ignore_pointer_events_none: false,
            },
            LayoutQuery::CaretPosition { point },
        ]);
        for ((width, height), expect_hit, expected_passes) in [
            (initial_size, false, 1),
            (expanded_size, true, 2),
            (expanded_size, true, 2),
        ] {
            if vm.layout_pass_observability_for_test().1 < before + expected_passes {
                vm.screenshot_layout_snapshot(LayoutViewport::new(width, height, 1.0))
                    .unwrap()
                    .unwrap();
            }
            let answers = GeometryProvider::answer(&mut *vm, &batch)
                .expect("point query batch after viewport expansion");
            assert_eq!(
                matches!(answers.answers[0], LayoutQueryAnswer::HitTest(Some(_))),
                expect_hit,
                "point {point:?} in {width}x{height} after {initial_size:?}"
            );
            let LayoutQueryAnswer::HitTestAll(hits) = &answers.answers[1] else {
                panic!("expected hit list")
            };
            assert_eq!(!hits.is_empty(), expect_hit);
            let LayoutQueryAnswer::CaretPosition(position) = &answers.answers[2] else {
                panic!("expected caret position")
            };
            assert_eq!(position.is_some(), expect_hit);
            assert_eq!(
                vm.layout_pass_observability_for_test().1,
                before + expected_passes,
                "the explicit output builds once; repeated queries consume its tree"
            );
        }
    }
}

#[test]
fn document_point_queries_retain_published_geometry_after_viewport_resize() {
    let mut vm = new_parsed_test_vm(
        "https://document-point-query-viewport-expansion.test/",
        "<html><body style='margin:0'><div id='target' style='width:50vw;height:100px'></div></body></html>",
    );
    let before = vm.layout_pass_observability_for_test().1;
    let mut completed_passes = before;
    let mut published = "[false,false,0]".to_owned();
    let query = r#"JSON.stringify([
        document.elementFromPoint(75, 20)?.id === 'target',
        document.elementsFromPoint(75, 20).some(element => element.id === 'target'),
        document.getElementById('target').getBoundingClientRect().width
    ])"#;
    for (width, expected, expected_passes) in [
        (100, "[false,false,50]", 1),
        (320, "[true,true,160]", 2),
        (100, "[false,false,50]", 3),
        (320, "[true,true,160]", 4),
        (320, "[true,true,160]", 4),
    ] {
        vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: width,
            inner_height: 200,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))
        .expect("point query viewport should update");
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            completed_passes,
            "changing the viewport alone must not trigger layout"
        );
        if completed_passes == before {
            assert_eq!(vm.eval(query).unwrap(), expected);
            completed_passes += 1;
        } else {
            assert_eq!(vm.eval(query).unwrap(), published);
        }
        assert_eq!(vm.layout_pass_observability_for_test().1, completed_passes);
        if completed_passes < before + expected_passes {
            vm.screenshot_layout_snapshot(moli_layout::LayoutViewport::new(width, 200, 1.0))
                .unwrap()
                .unwrap();
        }
        let result = vm
            .eval(query)
            .expect("document point queries after viewport resize");
        assert_eq!(result, expected, "viewport width {width}");
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            before + expected_passes,
            "only explicit output refreshes the viewport-dependent geometry"
        );
        completed_passes = before + expected_passes;
        published = result;
    }
}

#[test]
fn geometry_queries_retain_published_screen_and_resolution_environment() {
    // Exercise each entry point without first warming the other layout mode.
    for hit_test in [false, true] {
        let mut vm = new_parsed_test_vm(
            "https://geometry-screen-environment.test/",
            r#"<html><head><style>
                body { margin: 0 }
                #target { width: 100px; height: 100px }
                @media (device-width: 1280px), (device-height: 720px), (resolution: 2dppx) {
                    #target { width: 200px }
                }
            </style></head><body><div id=target></div></body></html>"#,
        );
        let mut surface = crate::protocol_types::ViewportSurface {
            inner_width: 800,
            inner_height: 600,
            device_pixel_ratio: 1.0,
            ..Default::default()
        };
        let before = vm.layout_pass_observability_for_test().1;
        let mut published_width = "0".to_owned();
        for (screen_width, screen_height, dpr, expected_width, expected_passes) in [
            (1920, 1080, 1.0, 100, 1),
            (1280, 1080, 1.0, 200, 2),
            (1280, 1080, 1.0, 200, 2),
            (1920, 1080, 1.0, 100, 3),
            (1920, 720, 1.0, 200, 4),
            (1920, 720, 1.0, 200, 4),
            (1920, 1080, 1.0, 100, 5),
            (1920, 1080, 2.0, 200, 6),
            (1920, 1080, 2.0, 200, 6),
            (1920, 1080, 1.0, 100, 7),
        ] {
            surface.screen_width = screen_width;
            surface.screen_height = screen_height;
            surface.device_pixel_ratio = dpr;
            // Moving the window changes the surface but not the style environment.
            surface.window_x += 1;
            let passes = vm.layout_pass_observability_for_test().1;
            let cache = vm.layout_snapshot_cache_observability_for_test();
            vm.set_viewport_surface(Some(surface))
                .expect("screen environment should update");
            assert_eq!(vm.layout_snapshot_cache_observability_for_test(), cache);
            assert_eq!(
                vm.layout_pass_observability_for_test().1,
                passes,
                "an environment update alone must not trigger layout"
            );
            if passes == before {
                published_width = expected_width.to_string();
            }
            assert_eq!(
                vm.eval("String(target.getBoundingClientRect().width)")
                    .unwrap(),
                published_width
            );
            let after_read = passes + u64::from(passes == before);
            assert_eq!(vm.layout_pass_observability_for_test().1, after_read);
            if after_read < before + expected_passes {
                vm.screenshot_layout_snapshot(moli_layout::LayoutViewport::new(
                    800, 600, dpr as f32,
                ))
                .unwrap()
                .unwrap();
            }
            let result = vm
                .eval(if hit_test {
                    r#"JSON.stringify([
                        document.elementFromPoint(150, 20)?.id === 'target',
                        document.elementsFromPoint(150, 20).some(element => element.id === 'target'),
                        document.getElementById('target').getBoundingClientRect().width
                    ])"#
                } else {
                    "String(document.getElementById('target').getBoundingClientRect().width)"
                })
                .expect("geometry query after screen environment update");
            published_width = expected_width.to_string();
            let hit = expected_width == 200;
            let expected = if hit_test {
                format!("[{hit},{hit},{expected_width}]")
            } else {
                expected_width.to_string()
            };
            assert_eq!(
                result, expected,
                "screen {screen_width}x{screen_height}, DPR {dpr}"
            );
            assert_eq!(
                vm.layout_pass_observability_for_test().1,
                before + expected_passes,
                "screen/resolution changes become visible only after explicit output"
            );
        }
    }
}

#[test]
fn geometry_queries_retain_published_media_environment() {
    use crate::protocol_types::EmulatedMediaOverrides;

    for overrides in [
        EmulatedMediaOverrides {
            media: Some("print".to_owned()),
            ..Default::default()
        },
        EmulatedMediaOverrides {
            color_scheme: Some("dark".to_owned()),
            ..Default::default()
        },
        EmulatedMediaOverrides {
            reduced_motion: Some("reduce".to_owned()),
            ..Default::default()
        },
    ] {
        let mut vm = new_parsed_test_vm(
            "https://geometry-media-environment.test/",
            r#"<html><head><style>
                body { margin: 0 }
                #target { width: 100px; height: 100px }
                @media print, (prefers-color-scheme: dark), (prefers-reduced-motion: reduce) {
                    #target { width: 200px }
                }
            </style></head><body><div id=target></div></body></html>"#,
        );
        let defaults = EmulatedMediaOverrides::default();
        let before = vm.layout_pass_observability_for_test().1;
        let mut published_width = "0".to_owned();
        for (environment, expected_width, expected_passes) in [
            (&defaults, 100, 1),
            (&overrides, 200, 2),
            (&overrides, 200, 2),
            (&defaults, 100, 3),
        ] {
            let passes = vm.layout_pass_observability_for_test().1;
            vm.set_emulated_media(environment);
            assert_eq!(vm.layout_pass_observability_for_test().1, passes);
            if passes == before {
                published_width = expected_width.to_string();
            }
            assert_eq!(
                vm.eval("String(target.getBoundingClientRect().width)")
                    .unwrap(),
                published_width
            );
            let after_read = passes + u64::from(passes == before);
            assert_eq!(vm.layout_pass_observability_for_test().1, after_read);
            if after_read < before + expected_passes {
                vm.screenshot_layout_snapshot(moli_layout::LayoutViewport::new(800, 600, 1.0))
                    .unwrap()
                    .unwrap();
            }
            let result = vm
                .eval(
                    r#"JSON.stringify([
                        document.elementFromPoint(150, 20)?.id === 'target',
                        document.getElementById('target').getBoundingClientRect().width
                    ])"#,
                )
                .expect("geometry query after media environment update");
            published_width = expected_width.to_string();
            assert_eq!(
                result,
                format!("[{},{}]", expected_width == 200, expected_width)
            );
            assert_eq!(
                vm.layout_pass_observability_for_test().1,
                before + expected_passes,
                "media changes become visible only after explicit output"
            );
        }
    }
}

#[test]
fn document_point_queries_parse_webidl_coordinates() {
    let mut vm = new_rendered_test_vm(
        "https://document-point-query-webidl.test/path/index.html",
        r#"<html><body>
            <div id="target" style="width:100px;height:50px">target</div>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && value.localName ? value.localName : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const arrayProbe = callback => {
    try {
      return callback().map((element) => element.localName).join(",");
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  return JSON.stringify({
    missingElement: probe(() => document.elementFromPoint()),
    missingElements: arrayProbe(() => document.elementsFromPoint(1)),
    symbolX: probe(() => document.elementFromPoint(Symbol(), 1)),
    symbolY: arrayProbe(() => document.elementsFromPoint(1, Symbol())),
    infinity: probe(() => document.elementFromPoint(Infinity, 1)),
    stringCoordinates: probe(() => document.elementFromPoint("10", "10")),
    objectCoordinates: arrayProbe(() => document.elementsFromPoint(
      { valueOf() { return 10; } },
      { valueOf() { return 10; } }
    ))
  });
})()
"#,
        )
        .expect("document point query WebIDL probe should evaluate");

    assert_eq!(
        result,
        r#"{"missingElement":"throw:TypeError","missingElements":"throw:TypeError","symbolX":"throw:TypeError","symbolY":"throw:TypeError","infinity":"throw:TypeError","stringCoordinates":"div","objectCoordinates":"div,body,html"}"#
    );
}

#[test]
fn shadow_root_point_queries_retarget_real_layout_hits_to_the_tree_scope() {
    let mut vm = new_rendered_test_vm(
        "https://shadow-root-point-query.test/path/index.html",
        r#"<html><head><style>
            html, body { margin: 0; }
            #host, #inside { display: block; width: 100px; height: 50px; }
        </style></head><body><div id="host"></div></body></html>"#,
    );

    let result = eval_with_layout_publications(
        &mut vm,
        r#"
(function* () {
  const host = document.getElementById('host');
  const shadow = host.attachShadow({ mode: 'closed' });
  shadow.innerHTML = '<span id="inside">text</span>';
yield; // Publish this scene before reading its geometry.
  return [
    document.elementFromPoint(1, 1)?.id,
    document.elementsFromPoint(1, 1).map(element => element.id || element.localName).join(','),
    shadow.elementFromPoint(1, 1)?.id,
    shadow.elementsFromPoint(1, 1).map(element => element.id || element.localName).join(',')
  ].join('|');
})()
"#,
    )
    .expect("shadow root point queries should evaluate");

    assert_eq!(result, "host|host,body,html|inside|inside,host,body,html");
}
