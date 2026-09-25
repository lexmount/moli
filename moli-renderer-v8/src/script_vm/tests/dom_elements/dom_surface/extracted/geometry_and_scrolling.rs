use super::*;

#[test]
fn element_scroll_into_view_surface_is_available() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const div = document.createElement("div");
              return JSON.stringify({
                instanceType: typeof div.scrollIntoView,
                protoType: typeof Element.prototype.scrollIntoView,
                inElement: "scrollIntoView" in div,
                ownProperty: Object.prototype.hasOwnProperty.call(div, "scrollIntoView"),
                callResult: String(div.scrollIntoView({ block: "center" })),
                protoEnumerable: Object.prototype.propertyIsEnumerable.call(Element.prototype, "scrollIntoView")
              });
            })()
            "#,
        )
        .expect("element scrollIntoView probe should evaluate");

    assert_eq!(
        result,
        r#"{"instanceType":"function","protoType":"function","inElement":true,"ownProperty":false,"callResult":"undefined","protoEnumerable":false}"#
    );
}
#[test]
fn wheel_default_action_scrolls_unless_canceled() {
    let mut vm = new_storage_test_vm("https://wheel-default-scroll.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.margin = "0";
          document.body.style.margin = "0";
          document.body.innerHTML =
            '<div style="height: 500px"></div>' +
            '<div id="marker" style="height: 20px"></div>' +
            '<div style="height: 2500px"></div>';
          window.__wheelDeltas = [];
          window.addEventListener("wheel", event => {
            window.__wheelDeltas.push(event.deltaY);
          }, { capture: true });
        })()
        "#,
    )
    .expect("wheel fixture should initialize");
    publish_layout_for_test(&mut vm);

    let before = vm
        .eval("document.getElementById('marker').getBoundingClientRect().top")
        .expect("initial marker geometry should evaluate")
        .parse::<f64>()
        .expect("initial marker top should be numeric");
    let outcome = vm
        .dispatch_mouse_event_at_point(10.0, 10.0, "wheel", -1, Some(0), 0.0, 120.0)
        .expect("wheel input should dispatch");
    assert!(outcome.handled);

    // Publish the scrolled geometry before checking its rendered position.
    publish_layout_for_test(&mut vm);
    let result = vm
        .eval(
            r#"
            JSON.stringify({
              scrollY,
              scrollingElementScrollTop: document.scrollingElement.scrollTop,
              markerTop: document.getElementById("marker").getBoundingClientRect().top,
              wheelDeltas: window.__wheelDeltas
            })
            "#,
        )
        .expect("post-wheel geometry should evaluate");
    let result: serde_json::Value =
        serde_json::from_str(&result).expect("post-wheel result should be JSON");
    assert_eq!(result["scrollY"], 120.0);
    assert_eq!(result["scrollingElementScrollTop"], 120.0);
    assert_eq!(result["markerTop"], before - 120.0);
    assert_eq!(result["wheelDeltas"], serde_json::json!([120]));

    vm.eval(
        r#"
        window.addEventListener("wheel", event => event.preventDefault(), {
          capture: true,
          passive: false
        })
        "#,
    )
    .expect("wheel cancellation listener should install");
    vm.dispatch_mouse_event_at_point(10.0, 10.0, "wheel", -1, Some(0), 0.0, 80.0)
        .expect("canceled wheel input should dispatch");
    assert_eq!(
        vm.eval("String(scrollY)")
            .expect("canceled wheel scroll position should evaluate"),
        "120"
    );
}
#[test]
fn intersection_checks_after_scroll_reuse_geometry_until_fresh_paint() {
    let mut vm = new_storage_test_vm("https://scroll-intersection-observer.test/");
    vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: 800,
        inner_height: 600,
        device_pixel_ratio: 1.0,
        ..Default::default()
    }))
    .expect("intersection observer viewport should match the layout fixture");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.margin = "0";
          document.body.style.margin = "0";
          document.body.innerHTML =
            '<div style="height: 800px"></div>' +
            '<div id="lazy-target" style="height: 20px"></div>' +
            '<div style="height: 1000px"></div>';
        })()
        "#,
    )
    .expect("intersection scroll fixture should initialize");
    publish_layout_for_test(&mut vm);

    vm.eval(
        r#"
        (() => {
          window.__intersectionStates = [];
          window.__intersectionObserver = new IntersectionObserver(entries => {
            window.__intersectionStates.push(entries[0].isIntersecting);
          });
          window.__intersectionObserver.observe(
            document.getElementById("lazy-target")
          );
        })()
        "#,
    )
    .expect("intersection observer should register");
    assert_eq!(
        vm.eval("JSON.stringify(window.__intersectionStates)")
            .expect("initial intersection state should flush"),
        "[false]"
    );

    let mut previous = "[false]";
    for (scroll_y, reason, expected) in [
        (
            400,
            moli_layout::LayoutFlushReason::Screenshot,
            "[false,true]",
        ),
        (
            0,
            moli_layout::LayoutFlushReason::Screencast,
            "[false,true,false]",
        ),
    ] {
        let passes_before = vm.layout_pass_observability_for_test().1;
        vm.eval(&format!("window.scrollTo(0, {scroll_y})"))
            .expect("window scroll should evaluate");
        assert_eq!(
            vm.eval("JSON.stringify(window.__intersectionStates)")
                .expect("scroll should retain the published intersection geometry"),
            previous
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, passes_before);
        vm.paint_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0), reason)
            .expect("fresh paint publishes the scrolled geometry")
            .expect("document layout");
        vm.eval("void 0")
            .expect("complete the turn and deliver queued observers");
        assert_eq!(
            vm.eval("JSON.stringify(window.__intersectionStates)")
                .expect("publication should queue observer delivery without another mutation"),
            expected
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, passes_before + 1);
        previous = expected;
    }
}
#[test]
fn wheel_default_action_scrolls_the_innermost_container_then_chains_to_the_root() {
    let mut vm = new_storage_test_vm("https://wheel-scroll-chain.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.margin = "0";
          document.body.style.margin = "0";
          document.body.innerHTML =
            '<div id="scroller" style="width: 200px; height: 100px; overflow: auto">' +
              '<div style="width: 500px; height: 500px"></div>' +
            '</div>' +
            '<div style="width: 2000px; height: 2000px"></div>';
        })()
        "#,
    )
    .expect("nested wheel fixture should initialize");
    publish_layout_for_test(&mut vm);

    vm.dispatch_mouse_event_at_point(10.0, 10.0, "wheel", -1, Some(0), 35.0, 60.0)
        .expect("nested wheel input should dispatch");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval(
            "JSON.stringify([document.getElementById('scroller').scrollLeft, document.getElementById('scroller').scrollTop, window.scrollX, window.scrollY])"
        )
            .expect("nested wheel positions should evaluate"),
        "[35,60,0,0]"
    );

    vm.eval(
        "document.getElementById('scroller').scrollTo(document.getElementById('scroller').scrollWidth, document.getElementById('scroller').scrollHeight)"
    )
    .expect("nested scroller should move to its boundary");
    vm.dispatch_mouse_event_at_point(10.0, 10.0, "wheel", -1, Some(0), 25.0, 40.0)
        .expect("chained wheel input should dispatch");
    assert_eq!(
        vm.eval("JSON.stringify([window.scrollX, window.scrollY])")
            .expect("root chained scroll position should evaluate"),
        "[25,40]"
    );
}
#[test]
fn classic_scrollbar_metrics_and_thumb_drag_match_chromium_without_dom_mouse_events() {
    let mut vm = new_storage_test_vm("https://classic-scrollbar-input.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.margin = "0";
          document.body.style.margin = "0";
          document.body.innerHTML = `
            <div id="scroller" style="width:200px;height:100px;overflow:auto">
              <div style="width:400px;height:300px"></div>
            </div>
            <div id="thin" style="width:200px;height:100px;overflow:scroll;scrollbar-width:thin">
              <div style="width:400px;height:300px"></div>
            </div>
            <div id="none" style="width:200px;height:100px;overflow:scroll;scrollbar-width:none">
              <div style="width:400px;height:300px"></div>
            </div>
            <div id="stable" style="width:200px;height:100px;overflow:auto;scrollbar-gutter:stable">
              <div style="height:20px"></div>
            </div>
            <div id="both" style="width:200px;height:100px;overflow:auto;scrollbar-gutter:stable both-edges">
              <div style="height:20px"></div>
            </div>
            <div id="feedback" style="width:200px;height:100px;overflow:auto">
              <div style="width:200px;height:200px"></div>
            </div>
            <div id="rtl" style="width:200px;height:100px;overflow:auto;direction:rtl;scrollbar-color:red blue">
              <div style="width:400px;height:300px"></div>
            </div>`;
          window.__scrollbarDomEvents = [];
          for (const name of ["pointerdown", "pointermove", "pointerup", "mousedown", "mousemove", "mouseup", "click"]) {
            document.addEventListener(name, () => window.__scrollbarDomEvents.push(name), true);
          }
        })()
        "#,
    )
    .expect("scrollbar fixture should initialize");
    publish_layout_for_test(&mut vm);

    assert_eq!(
        vm.eval(
            r#"JSON.stringify([
              scroller.clientWidth, scroller.clientHeight,
              thin.clientWidth, thin.clientHeight,
              none.clientWidth, none.clientHeight,
              scroller.scrollWidth, scroller.scrollHeight,
              stable.clientWidth, stable.clientLeft, stable.firstElementChild.offsetWidth,
              Math.round(stable.firstElementChild.getBoundingClientRect().left),
              both.clientWidth, both.clientLeft, both.firstElementChild.offsetWidth,
              Math.round(both.firstElementChild.getBoundingClientRect().left),
              feedback.clientWidth, feedback.clientHeight,
              rtl.clientWidth, rtl.clientLeft, rtl.scrollWidth, rtl.scrollLeft
            ])"#,
        )
        .expect("classic scrollbar metrics should evaluate"),
        "[185,85,190,90,200,100,400,300,185,0,185,0,170,15,170,15,185,85,185,15,400,0]"
    );

    vm.dispatch_mouse_event_at_point(190.0, 30.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("vertical thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(190.0, 50.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("vertical thumb drag should dispatch");
    vm.dispatch_mouse_event_at_point(190.0, 50.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("vertical thumb release should dispatch");

    vm.dispatch_mouse_event_at_point(50.0, 90.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("horizontal thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(90.0, 90.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("horizontal thumb drag should dispatch");
    vm.dispatch_mouse_event_at_point(90.0, 90.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("horizontal thumb release should dispatch");

    publish_layout_for_test(&mut vm);
    let before_controls = vm
        .eval("scroller.scrollTop")
        .expect("pre-control scrollTop should evaluate")
        .parse::<f64>()
        .expect("pre-control scrollTop should be numeric");
    vm.dispatch_mouse_event_at_point(190.0, 5.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("back button press should dispatch");
    vm.dispatch_mouse_event_at_point(190.0, 5.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("back button release should dispatch");
    publish_layout_for_test(&mut vm);
    let after_back = vm
        .eval("scroller.scrollTop")
        .expect("back-button scrollTop should evaluate")
        .parse::<f64>()
        .expect("back-button scrollTop should be numeric");
    assert!((after_back - (before_controls - 40.0)).abs() < 0.01);

    vm.dispatch_mouse_event_at_point(190.0, 80.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("forward button press should dispatch");
    vm.dispatch_mouse_event_at_point(190.0, 80.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("forward button release should dispatch");
    publish_layout_for_test(&mut vm);
    let after_forward = vm
        .eval("scroller.scrollTop")
        .expect("forward-button scrollTop should evaluate")
        .parse::<f64>()
        .expect("forward-button scrollTop should be numeric");
    assert!((after_forward - before_controls).abs() < 0.01);

    vm.dispatch_mouse_event_at_point(190.0, 60.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("forward track press should dispatch");
    vm.dispatch_mouse_event_at_point(190.0, 60.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("forward track release should dispatch");
    publish_layout_for_test(&mut vm);
    let after_track = vm
        .eval("scroller.scrollTop")
        .expect("track scrollTop should evaluate")
        .parse::<f64>()
        .expect("track scrollTop should be numeric");
    assert!((after_track - (before_controls + 85.0 * 0.875)).abs() < 0.01);

    let result = vm
        .eval(
            r#"JSON.stringify({
              left: scroller.scrollLeft,
              top: scroller.scrollTop,
              events: window.__scrollbarDomEvents
            })"#,
        )
        .expect("post-drag scroll state should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).expect("scroll result JSON");
    assert!(result["left"].as_f64().is_some_and(|value| value > 50.0));
    assert!(result["top"].as_f64().is_some_and(|value| value > 100.0));
    assert_eq!(result["events"], serde_json::json!([]));
}
#[test]
fn painted_overlay_wins_over_scrollbar_and_corner_consumes_input() {
    let mut vm = new_storage_test_vm("https://painted-scrollbar-surface.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.margin = "0";
          document.body.style.margin = "0";
          document.body.innerHTML = `
            <div id="scroller" style="position:absolute;left:20px;top:20px;width:200px;height:100px;overflow:scroll">
              <div style="width:400px;height:300px"></div>
            </div>
            <div id="overlay" style="position:absolute;left:200px;top:20px;width:40px;height:100px;z-index:10"></div>`;
          window.__paintedSurfaceEvents = [];
          for (const name of ["pointerdown", "mousedown", "pointerup", "mouseup", "click"]) {
            document.addEventListener(name, event => {
              window.__paintedSurfaceEvents.push(`${name}:${event.target.id || event.target.localName}`);
            }, true);
          }
        })()
        "#,
    )
    .expect("painted surface fixture should initialize");
    publish_layout_for_test(&mut vm);
    vm.eval("scroller.scrollTop = 80")
        .expect("scroller should move before the overlay probe");
    publish_layout_for_test(&mut vm);

    vm.dispatch_mouse_event_at_point(210.0, 30.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("overlay press should dispatch");
    vm.dispatch_mouse_event_at_point(210.0, 30.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("overlay release should dispatch");
    assert_eq!(
        vm.eval("JSON.stringify([scroller.scrollTop, __paintedSurfaceEvents])")
            .expect("overlay result should evaluate"),
        r#"[80,["pointerdown:overlay","mousedown:overlay","pointerup:overlay","mouseup:overlay","click:overlay"]]"#,
        "a higher painted sibling must occlude the scrollbar beneath it"
    );

    vm.eval(
        r#"
        overlay.style.display = "none";
        window.__paintedSurfaceEvents.length = 0;
        "#,
    )
    .expect("overlay should be removed before the corner probe");
    publish_layout_for_test(&mut vm);
    let corner = vm
        ._context_host
        .borrow()
        .with_latest_layout_tree_for_document(vm.document_runtime.document_handle(), |tree| {
            tree.control_surface_hit_test(moli_layout::LayoutPoint::new(210.0, 110.0))
        })
        .flatten();
    assert!(
        matches!(
            corner,
            Some(moli_layout::LayoutControlSurfaceHit::ScrollbarCorner(_))
        ),
        "the painted lower-right scrollbar corner should be an input surface: {corner:?}"
    );

    vm.dispatch_mouse_event_at_point(210.0, 110.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("corner press should be consumed");
    vm.dispatch_mouse_event_at_point(210.0, 110.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("corner release should be consumed");
    assert_eq!(
        vm.eval("JSON.stringify([scroller.scrollTop, __paintedSurfaceEvents])")
            .expect("corner result should evaluate"),
        "[80,[]]",
        "the UA scrollbar corner must not fall through into DOM dispatch"
    );
}
#[test]
fn cold_geometry_entries_share_one_layout_and_do_not_rebuild_for_missing_boxes() {
    for query in [
        "target.clientWidth",
        "target.getBoundingClientRect().width",
        "target.getClientRects()[0].width",
        "range.getBoundingClientRect().width",
        "document.elementFromPoint(30, 30) === target ? 100 : -1",
    ] {
        let mut vm = new_parsed_test_vm(
            "https://initial-geometry.test/",
            "<!doctype html><div id=target style='position:absolute;left:20px;top:20px;width:100px;height:80px'></div><div id=hidden style='display:none'></div>",
        );
        vm.eval("globalThis.range=document.createRange();range.selectNode(target)")
            .unwrap();
        let before = vm.layout_pass_observability_for_test().1;
        assert_eq!(vm.eval(query).unwrap(), "100", "{query}");
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
        vm.eval("target.style.width='200px';const added=document.createElement('div');added.id='added';added.style.width='50px';document.body.appendChild(added)").unwrap();
        assert_eq!(
            vm.eval("JSON.stringify([target.clientWidth,hidden.clientWidth,added.clientWidth])")
                .unwrap(),
            "[100,0,0]"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
        publish_layout_for_test(&mut vm);
        assert_eq!(
            vm.eval("JSON.stringify([target.clientWidth,hidden.clientWidth,added.clientWidth])")
                .unwrap(),
            "[200,0,50]"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
    }
}
#[test]
fn document_client_size_initializes_layout_once_and_reuses_it_until_capture() {
    let mut vm = new_parsed_test_vm(
        "https://document-client-viewport.test/",
        "<!doctype html><style>html,body{margin:0}main{width:80px;height:900px}</style><main></main>",
    );
    let initial_passes = vm.layout_pass_observability_for_test().1;
    let query = r#"JSON.stringify([
        document.documentElement.clientWidth, document.documentElement.clientHeight,
        document.documentElement.offsetWidth, document.documentElement.scrollHeight,
        document.body.clientWidth, document.querySelector('main').clientHeight,
        document.querySelector('main').getBoundingClientRect().width
    ])"#;
    vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: 480,
        inner_height: 360,
        device_pixel_ratio: 2.0,
        ..Default::default()
    }))
    .unwrap();
    let published = vm.eval(query).unwrap();
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        initial_passes + 1
    );
    let published_metrics: serde_json::Value = serde_json::from_str(&published).unwrap();
    assert_eq!(published_metrics[0], 465); // The viewport now has a scrollbar.
    assert_eq!(published_metrics[1], 360);
    assert_eq!(published_metrics[3], 900);
    assert_eq!(published_metrics[6], 80);
    let published_passes = vm.layout_pass_observability_for_test().1;

    vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: 640,
        inner_height: 480,
        ..Default::default()
    }))
    .unwrap();
    assert_eq!(vm.eval(query).unwrap(), published);
    assert_eq!(vm.layout_pass_observability_for_test().1, published_passes);

    vm.eval("document.documentElement.style.display = 'none'")
        .unwrap();
    publish_layout_for_test(&mut vm);
    assert_eq!(vm.eval(query).unwrap(), "[0,0,0,0,0,0,0]");
}
#[test]
fn body_overflow_defines_viewport_scrolling_and_default_root_stable_gutters() {
    let mut vm = new_storage_test_vm("https://viewport-overflow-policy.test/");
    vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: 800,
        inner_height: 600,
        outer_width: 800,
        outer_height: 600,
        device_pixel_ratio: 1.0,
        screen_width: 800,
        screen_height: 600,
        screen_avail_width: 800,
        screen_avail_height: 600,

        ..Default::default()
    }))
    .expect("viewport overflow surface should update");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.cssText = "margin:0;overflow:visible";
          document.body.style.cssText = "margin:0;overflow:hidden";
          document.body.innerHTML = '<main id="content" style="height:1200px"></main>';
        })()
        "#,
    )
    .expect("viewport overflow fixture should initialize");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval(
            "JSON.stringify([document.documentElement.clientWidth, document.documentElement.scrollHeight, scrollY])"
        )
        .expect("hidden viewport metrics should evaluate"),
        "[800,1200,0]"
    );

    vm.dispatch_mouse_event_at_point(10.0, 10.0, "wheel", -1, Some(0), 0.0, 80.0)
        .expect("hidden viewport wheel should dispatch without scrolling");
    assert_eq!(
        vm.eval("String(scrollY)")
            .expect("hidden viewport wheel result should evaluate"),
        "0"
    );
    vm.eval("scrollTo(0, 100)")
        .expect("hidden viewport should remain script-scrollable");
    assert_eq!(
        vm.eval("String(scrollY)")
            .expect("hidden programmatic scroll should evaluate"),
        "100"
    );

    vm.eval("document.body.style.overflow = 'auto'; scrollTo(0, 0)")
        .expect("body auto should become the viewport policy");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("String(document.documentElement.clientWidth)")
            .expect("auto viewport width should evaluate"),
        "785"
    );
    vm.dispatch_mouse_event_at_point(10.0, 10.0, "wheel", -1, Some(0), 0.0, 80.0)
        .expect("auto viewport wheel should scroll");
    assert_eq!(
        vm.eval("String(scrollY)")
            .expect("auto viewport wheel result should evaluate"),
        "80"
    );

    vm.eval("document.body.style.overflow = 'clip'")
        .expect("body clip should become hidden at the viewport boundary");
    publish_layout_for_test(&mut vm);
    vm.eval("scrollTo(0, 100)")
        .expect("clip-derived viewport should remain script-scrollable");
    vm.dispatch_mouse_event_at_point(10.0, 10.0, "wheel", -1, Some(0), 0.0, 80.0)
        .expect("clip-derived viewport wheel should not scroll");
    assert_eq!(
        vm.eval("JSON.stringify([document.documentElement.clientWidth, scrollY])")
            .expect("clip-derived viewport result should evaluate"),
        "[800,100]"
    );

    vm.eval(
        r#"
        document.documentElement.style.cssText =
          "margin:0;overflow:visible;scrollbar-gutter:stable";
        document.body.style.cssText = "margin:0;overflow:visible";
        document.body.innerHTML = "";
        "#,
    )
    .expect("stable viewport gutter fixture should initialize");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify((() => {
              const html = document.documentElement.getBoundingClientRect();
              const body = document.body.getBoundingClientRect();
              return [document.documentElement.clientWidth, html.x, html.width, body.x, body.width];
            })())
            "#,
        )
        .expect("default-visible stable viewport gutter should evaluate"),
        "[800,0,785,0,785]"
    );

    vm.eval("document.documentElement.style.scrollbarGutter = 'stable both-edges'")
        .expect("both-edge stable gutter should apply");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify((() => {
              const html = document.documentElement.getBoundingClientRect();
              const body = document.body.getBoundingClientRect();
              return [document.documentElement.clientWidth, html.x, html.width, body.x, body.width];
            })())
            "#,
        )
        .expect("default-visible both-edge viewport gutter should evaluate"),
        "[800,15,770,15,770]"
    );

    vm.eval(
        r#"
        document.documentElement.style.cssText = "margin:0;overflow:visible";
        document.body.style.cssText = "display:contents;overflow:scroll";
        document.body.innerHTML = "";
        "#,
    )
    .expect("display-contents body fixture should initialize");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("String(document.documentElement.getBoundingClientRect().width)")
            .expect("display-contents viewport width should evaluate"),
        "800",
        "overflow-body-propagation-003: a body without a principal box cannot define the viewport"
    );

    vm.eval("document.body.style.display = 'block'")
        .expect("principal body fixture should initialize");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("String(document.documentElement.getBoundingClientRect().width)")
            .expect("principal-body viewport width should evaluate"),
        "785",
        "a principal body with overflow:scroll must define the viewport"
    );
}
#[test]
fn physical_scrollbar_insets_cover_box_intrinsic_ratio_and_vertical_writing_layout() {
    let mut vm = new_storage_test_vm("https://physical-scrollbar-insets.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.margin = "0";
          document.body.style.margin = "0";
          document.body.innerHTML = `
            <style>
              .case {
                margin: 0;
                overflow: scroll;
                scrollbar-gutter: stable both-edges;
              }
              .case > div { width: 100%; height: 100%; }
              #border {
                box-sizing: border-box;
                width: 200px; height: 100px;
                padding: 10px; border: 5px solid;
              }
              #content {
                box-sizing: content-box;
                width: 200px; height: 100px;
                padding: 10px; border: 5px solid;
              }
              .auto {
                box-sizing: border-box;
                width: 200px; height: auto;
                min-height: 80px; max-height: 120px;
              }
              #auto-small > div { width: 400px; height: 20px; }
              #auto-mid > div { width: 400px; height: 90px; }
              #auto-large > div { width: 400px; height: 150px; }
              #ratio {
                box-sizing: border-box;
                width: 200px; height: auto;
                aspect-ratio: 2;
              }
              #vertical {
                box-sizing: border-box;
                width: 200px; height: 100px;
                writing-mode: vertical-rl;
              }
            </style>
            <div id="border" class="case"><div></div></div>
            <div id="content" class="case"><div></div></div>
            <div id="auto-small" class="case auto"><div></div></div>
            <div id="auto-mid" class="case auto"><div></div></div>
            <div id="auto-large" class="case auto"><div></div></div>
            <div id="ratio" class="case"><div></div></div>
            <div id="vertical" class="case"><div></div></div>`;
        })()
        "#,
    )
    .expect("physical scrollbar inset fixture should initialize");
    publish_layout_for_test(&mut vm);

    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify([
              "border", "content", "auto-small", "auto-mid", "auto-large", "ratio", "vertical"
            ].map(id => {
              const scroller = document.getElementById(id);
              const child = scroller.firstElementChild;
              const outer = scroller.getBoundingClientRect();
              const inner = child.getBoundingClientRect();
              return [
                scroller.offsetWidth, scroller.offsetHeight,
                scroller.clientWidth, scroller.clientHeight,
                scroller.clientLeft, scroller.clientTop,
                inner.x - outer.x, inner.y - outer.y, inner.width, inner.height,
              ];
            }))
            "#,
        )
        .expect("physical scrollbar inset metrics should evaluate"),
        "[[200,100,160,75,20,5,30,15,140,55],[230,130,190,105,20,5,30,15,170,85],[200,80,170,65,15,0,15,0,400,20],[200,105,170,90,15,0,15,0,400,90],[200,120,170,105,15,0,15,0,400,150],[200,100,170,85,15,0,15,0,170,85],[200,100,185,70,0,15,0,15,185,70]]"
    );
}
#[test]
fn nested_and_sibling_scrollbar_drags_stay_bound_to_the_pressed_scroller() {
    let mut vm = new_storage_test_vm("https://nested-classic-scrollbar-input.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.margin = "0";
          document.body.style.margin = "0";
          document.body.innerHTML = `
            <div id="outer" style="position:absolute;left:20px;top:20px;width:280px;height:200px;overflow:auto">
              <div style="position:relative;width:620px;height:500px">
                <div id="inner" style="position:absolute;left:30px;top:30px;width:180px;height:120px;overflow:auto">
                  <div style="width:420px;height:320px"></div>
                </div>
              </div>
            </div>
            <div id="sibling" style="position:absolute;left:340px;top:20px;width:180px;height:120px;overflow:auto">
              <div style="width:420px;height:320px"></div>
            </div>
            <div id="horizontal" style="position:absolute;left:340px;top:180px;width:220px;height:100px;overflow:auto">
              <div style="width:600px;height:60px"></div>
            </div>
            <div id="thin-child" style="position:absolute;left:600px;top:20px;width:160px;height:120px;overflow:auto;scrollbar-width:thin">
              <div style="width:400px;height:320px"></div>
            </div>
            <div id="rtl-child" style="position:absolute;left:600px;top:180px;width:160px;height:120px;overflow:auto;direction:rtl">
              <div style="width:400px;height:320px"></div>
            </div>`;
          window.__nestedScrollbarDomEvents = [];
          for (const name of ["pointerdown", "pointermove", "pointerup", "mousedown", "mousemove", "mouseup", "click"]) {
            document.addEventListener(name, () => __nestedScrollbarDomEvents.push(name), true);
          }
        })()
        "#,
    )
    .expect("nested scrollbar fixture should initialize");
    publish_layout_for_test(&mut vm);

    assert_eq!(
        vm.eval(
            r#"JSON.stringify([
              outer.clientWidth, outer.clientHeight, outer.scrollWidth, outer.scrollHeight,
              inner.clientWidth, inner.clientHeight, inner.scrollWidth, inner.scrollHeight,
              sibling.clientWidth, sibling.clientHeight,
              horizontal.clientWidth, horizontal.clientHeight,
              horizontal.scrollWidth, horizontal.scrollHeight,
              document.getElementById("thin-child").clientWidth,
              document.getElementById("thin-child").clientHeight,
              document.getElementById("rtl-child").clientWidth,
              document.getElementById("rtl-child").clientHeight,
              document.getElementById("rtl-child").clientLeft,
              document.getElementById("rtl-child").scrollLeft
            ])"#,
        )
        .expect("multi-scroller metrics should evaluate"),
        "[265,185,620,500,165,105,420,320,165,105,220,85,600,85,150,110,145,105,15,0]"
    );

    // Start on the nested vertical thumb, then move and release over the
    // sibling's scrollbar. Native dragging captures the originally pressed
    // scrollbar instead of retargeting every move by viewport hit testing.
    vm.dispatch_mouse_event_at_point(225.0, 75.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("nested thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(515.0, 115.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("captured nested thumb move should dispatch");
    vm.dispatch_mouse_event_at_point(515.0, 115.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("captured nested thumb release should dispatch");
    publish_layout_for_test(&mut vm);
    let after_nested: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify([inner.scrollTop, outer.scrollTop, sibling.scrollTop])")
            .expect("nested drag state should evaluate"),
    )
    .expect("nested drag state should be JSON");
    assert!(
        after_nested[0].as_f64().is_some_and(|value| value > 180.0),
        "nested thumb should move its own element: {after_nested}"
    );
    assert_eq!(after_nested[1], 0);
    assert_eq!(after_nested[2], 0);

    vm.dispatch_mouse_event_at_point(515.0, 45.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("sibling thumb press should dispatch after captured release");
    vm.dispatch_mouse_event_at_point(515.0, 85.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("sibling thumb move should dispatch");
    vm.dispatch_mouse_event_at_point(515.0, 85.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("sibling thumb release should dispatch");

    vm.dispatch_mouse_event_at_point(370.0, 275.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("horizontal child thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(450.0, 275.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("horizontal child thumb move should dispatch");
    vm.dispatch_mouse_event_at_point(450.0, 275.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("horizontal child thumb release should dispatch");

    vm.dispatch_mouse_event_at_point(755.0, 45.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("thin vertical thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(755.0, 75.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("thin vertical thumb move should dispatch");
    vm.dispatch_mouse_event_at_point(755.0, 75.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("thin vertical thumb release should dispatch");
    vm.dispatch_mouse_event_at_point(630.0, 135.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("thin horizontal thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(690.0, 135.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("thin horizontal thumb move should dispatch");
    vm.dispatch_mouse_event_at_point(690.0, 135.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("thin horizontal thumb release should dispatch");

    vm.dispatch_mouse_event_at_point(607.0, 205.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("RTL left-edge vertical thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(607.0, 235.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("RTL left-edge vertical thumb move should dispatch");
    vm.dispatch_mouse_event_at_point(607.0, 235.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("RTL left-edge vertical thumb release should dispatch");
    vm.dispatch_mouse_event_at_point(720.0, 292.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("RTL horizontal thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(660.0, 292.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("RTL horizontal thumb move should dispatch toward the negative range");
    vm.dispatch_mouse_event_at_point(660.0, 292.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("RTL horizontal thumb release should dispatch");

    // Move the nested scroller partly above its parent's clip, then prove the
    // remaining visible piece of its thumb is hittable in the new space.
    vm.eval("outer.scrollTop = 50; inner.scrollTop = 0")
        .expect("ancestor and nested scroll positions should reset");
    publish_layout_for_test(&mut vm);
    vm.dispatch_mouse_event_at_point(225.0, 25.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("partly clipped nested thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(700.0, 45.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("partly clipped captured move should dispatch outside the scroller");
    vm.dispatch_mouse_event_at_point(700.0, 45.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("partly clipped captured release should dispatch");

    vm.dispatch_mouse_event_at_point(295.0, 65.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("outer thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(295.0, 125.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("outer thumb move should dispatch");
    vm.dispatch_mouse_event_at_point(295.0, 125.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("outer thumb release should dispatch");

    publish_layout_for_test(&mut vm);
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval(
            "JSON.stringify([outer.scrollTop, inner.scrollTop, sibling.scrollTop, horizontal.scrollLeft, document.getElementById('thin-child').scrollLeft, document.getElementById('thin-child').scrollTop, document.getElementById('rtl-child').scrollLeft, document.getElementById('rtl-child').scrollTop, __nestedScrollbarDomEvents])",
        )
        .expect("multi-scroller drag state should evaluate"),
    )
    .expect("multi-scroller drag state should be JSON");
    assert!(result[0].as_f64().is_some_and(|value| value > 200.0));
    assert!(result[1].as_f64().is_some_and(|value| value > 90.0));
    assert!(result[2].as_f64().is_some_and(|value| value > 180.0));
    assert!(result[3].as_f64().is_some_and(|value| value > 250.0));
    assert!(result[4].as_f64().is_some_and(|value| value > 180.0));
    assert!(result[5].as_f64().is_some_and(|value| value > 100.0));
    assert!(result[6].as_f64().is_some_and(|value| value < -180.0));
    assert!(result[7].as_f64().is_some_and(|value| value > 130.0));
    assert_eq!(result[8], serde_json::json!([]));
}
#[test]
fn transformed_scrollbar_drag_uses_local_motion_clamps_and_cancels_on_button_loss() {
    let mut vm = new_storage_test_vm("https://transformed-classic-scrollbar-input.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.margin = "0";
          document.body.style.margin = "0";
          document.body.innerHTML = `
            <div id="scaled" style="position:absolute;left:50px;top:350px;width:160px;height:100px;overflow:auto;transform:scale(2);transform-origin:0 0">
              <div style="width:400px;height:300px"></div>
            </div>
            <div id="releaseTarget" style="position:absolute;left:650px;top:350px;width:140px;height:220px"></div>`;
          window.__transformedScrollbarDomEvents = [];
          for (const name of ["pointermove", "mousemove"]) {
            document.addEventListener(name, () => __transformedScrollbarDomEvents.push(name), true);
          }
        })()
        "#,
    )
    .expect("transformed scrollbar fixture should initialize");
    publish_layout_for_test(&mut vm);

    let hit = crate::native_bridge::element::observable_scrollbar_hit_test(
        &vm._context_host.borrow(),
        vm.document_runtime.document_handle(),
        moli_layout::LayoutPoint::new(355.0, 400.0),
    )
    .expect("transformed scrollbar hit test should succeed")
    .expect("scaled vertical thumb should be hit");
    assert_eq!(hit.part, moli_layout::LayoutScrollbarPart::Thumb);
    assert_eq!(
        hit.scrollbar.axis,
        moli_layout::LayoutScrollbarAxis::Vertical
    );

    // Cross-axis motion does nothing. Each later move is absolute from the
    // original press, and forty viewport pixels are twenty local pixels under
    // scale(2), rather than forty CSS pixels or a sum of prior move deltas.
    vm.dispatch_mouse_event_at_point(355.0, 400.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("scaled thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(700.0, 400.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("scaled cross-axis-only captured move should dispatch");
    assert_eq!(
        vm.eval("scaled.scrollTop")
            .expect("cross-axis-only scaled scrollTop should evaluate"),
        "0"
    );
    vm.dispatch_mouse_event_at_point(700.0, 420.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("first scaled incremental thumb move should dispatch");
    publish_layout_for_test(&mut vm);
    let first_move = vm
        .eval("scaled.scrollTop")
        .expect("first scaled scrollTop should evaluate")
        .parse::<f64>()
        .expect("first scaled scrollTop should be numeric");
    assert!(
        (first_move - 67.1875).abs() < 0.01,
        "scrollTop={first_move}"
    );
    vm.dispatch_mouse_event_at_point(700.0, 440.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("scaled captured thumb move should dispatch outside its element");
    vm.dispatch_mouse_event_at_point(700.0, 440.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("scaled captured thumb release should dispatch");
    publish_layout_for_test(&mut vm);
    let moderate = vm
        .eval("scaled.scrollTop")
        .expect("scaled scrollTop should evaluate")
        .parse::<f64>()
        .expect("scaled scrollTop should be numeric");
    assert!((moderate - 134.375).abs() < 0.01, "scrollTop={moderate}");

    vm.eval("scaled.scrollTop = 0")
        .expect("scaled scrollTop should reset");
    publish_layout_for_test(&mut vm);
    vm.dispatch_mouse_event_at_point(355.0, 400.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("scaled lower-clamp thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(700.0, 590.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("scaled lower-clamp thumb move should dispatch");
    vm.dispatch_mouse_event_at_point(700.0, 590.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("scaled lower-clamp thumb release should dispatch");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("scaled.scrollTop")
            .expect("maximum scaled scrollTop should evaluate"),
        "215"
    );

    vm.dispatch_mouse_event_at_point(355.0, 460.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("scaled upper-clamp thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(700.0, 300.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("scaled upper-clamp thumb move should dispatch");
    vm.dispatch_mouse_event_at_point(700.0, 300.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("scaled upper-clamp thumb release should dispatch");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("scaled.scrollTop")
            .expect("minimum scaled scrollTop should evaluate"),
        "0"
    );

    // Losing the primary-button bit cancels native capture. Later moves must
    // return to DOM dispatch and must not continue scrolling the old element.
    vm.dispatch_mouse_event_at_point(355.0, 400.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("cancelable scaled thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(700.0, 440.0, "mousemove", -1, Some(0), 0.0, 0.0)
        .expect("primary-button loss should cancel native drag");
    vm.dispatch_mouse_event_at_point(700.0, 500.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("post-cancel DOM move should dispatch");
    let cancelled: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify([scaled.scrollTop, __transformedScrollbarDomEvents])")
            .expect("cancelled transformed drag state should evaluate"),
    )
    .expect("cancelled transformed drag state should be JSON");
    assert_eq!(cancelled[0], 0);
    assert!(
        cancelled[1]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event == "mousemove")),
        "post-cancel moves should return to the DOM: {cancelled}"
    );
}
#[test]
fn document_replacement_cancels_old_scrollbar_capture_and_allows_a_new_drag() {
    let mut vm = new_storage_test_vm("https://replacement-classic-scrollbar-input.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.margin = "0";
          document.body.style.margin = "0";
          document.body.innerHTML = `
            <div id="retiringScroller" style="width:200px;height:100px;overflow:auto">
              <div style="width:400px;height:300px"></div>
            </div>`;
          window.__retiredScroller = retiringScroller;
        })()
        "#,
    )
    .expect("retiring scrollbar fixture should initialize");
    publish_layout_for_test(&mut vm);
    vm.dispatch_mouse_event_at_point(190.0, 30.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("retiring thumb press should dispatch");

    vm.eval(
        r#"
        document.open();
        document.write(`<!doctype html><style>html,body{margin:0}</style>
          <div id="replacementScroller" style="width:200px;height:100px;overflow:auto">
            <div style="width:400px;height:300px"></div>
          </div>
          <div id="replacementTarget" style="position:absolute;left:300px;top:0;width:100px;height:100px"></div>`);
        document.close();
        window.__replacementMoves = [];
        replacementTarget.addEventListener("pointermove", () => __replacementMoves.push("pointermove"));
        replacementTarget.addEventListener("mousemove", () => __replacementMoves.push("mousemove"));
        "#,
    )
    .expect("document.open should install the replacement input fixture");
    publish_layout_for_test(&mut vm);

    vm.dispatch_mouse_event_at_point(320.0, 20.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("replacement-document move should dispatch normally");
    let replacement_move: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify([__retiredScroller.scrollTop, __replacementMoves])")
            .expect("replacement move state should evaluate"),
    )
    .expect("replacement move state should be JSON");
    assert_eq!(replacement_move[0], 0);
    assert_eq!(
        replacement_move[1],
        serde_json::json!(["pointermove", "mousemove"])
    );

    vm.dispatch_mouse_event_at_point(190.0, 30.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("replacement thumb press should start a fresh drag");
    vm.dispatch_mouse_event_at_point(190.0, 50.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("replacement thumb move should dispatch");
    vm.dispatch_mouse_event_at_point(190.0, 50.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("replacement thumb release should dispatch");
    publish_layout_for_test(&mut vm);
    let replacement_scroll = vm
        .eval("replacementScroller.scrollTop")
        .expect("replacement scroller state should evaluate")
        .parse::<f64>()
        .expect("replacement scrollTop should be numeric");
    assert!(replacement_scroll > 130.0, "scrollTop={replacement_scroll}");
}
#[test]
fn closed_absolute_popover_does_not_expand_root_scrollable_overflow() {
    let mut vm = new_storage_test_vm("https://closed-popover-overflow.test/");
    vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: 800,
        inner_height: 600,
        outer_width: 800,
        outer_height: 600,
        device_pixel_ratio: 1.0,
        screen_width: 1920,
        screen_height: 1080,
        screen_avail_width: 1920,
        screen_avail_height: 1040,

        ..Default::default()
    }))
    .expect("popover overflow viewport surface should update");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.margin = "0";
          document.body.style.margin = "0";
          document.body.innerHTML = `
            <nav style="overflow:hidden;width:100%;height:40px">
              <tool-tip id="tip" popover="manual"
                style="position:absolute;left:900px;width:200px;height:20px">
                Tooltip
              </tool-tip>
            </nav>
            <main style="height:1200px"></main>`;
        })()
        "#,
    )
    .expect("closed popover overflow fixture should initialize");

    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify([tip.getBoundingClientRect().width, document.documentElement.scrollWidth])")
            .expect("closed popover geometry should evaluate"),
        "[0,785]"
    );

    vm.eval("tip.showPopover()").expect("popover should open");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify([tip.getBoundingClientRect().width, document.documentElement.scrollWidth])")
            .expect("open popover geometry should evaluate"),
        "[200,1100]"
    );

    vm.eval("tip.hidePopover()").expect("popover should close");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify([tip.getBoundingClientRect().width, document.documentElement.scrollWidth])")
            .expect("reclosed popover geometry should evaluate"),
        "[0,785]"
    );
}
#[test]
fn root_scrollbar_gutters_size_the_initial_containing_block_once() {
    let mut vm = new_storage_test_vm("https://root-scrollbar-containing-block.test/");
    vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: 800,
        inner_height: 600,
        outer_width: 800,
        outer_height: 600,
        device_pixel_ratio: 1.0,
        screen_width: 1920,
        screen_height: 1080,
        screen_avail_width: 1920,
        screen_avail_height: 1040,

        ..Default::default()
    }))
    .expect("root gutter viewport surface should update");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.cssText = "margin:0;height:100%";
          document.body.style.cssText = "margin:0;height:100%";
          document.body.innerHTML = `
            <div id="wide" style="width:1600px;height:50%"></div>
            <div id="fixed" style="position:fixed;right:0;bottom:0;width:10px;height:10px"></div>`;
        })()
        "#,
    )
    .expect("percentage root overflow fixture should initialize");
    publish_layout_for_test(&mut vm);

    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify((() => {
              const html = document.documentElement.getBoundingClientRect();
              const body = document.body.getBoundingClientRect();
              const wideRect = wide.getBoundingClientRect();
              const fixedRect = fixed.getBoundingClientRect();
              return [
                document.documentElement.clientWidth,
                document.documentElement.clientHeight,
                document.documentElement.scrollWidth,
                document.documentElement.scrollHeight,
                html.x, html.width, html.height,
                body.x, body.width, body.height,
                wideRect.width, wideRect.height,
                fixedRect.x, fixedRect.y
              ];
            })())
            "#,
        )
        .expect("percentage root overflow metrics should evaluate"),
        "[800,585,1600,585,0,800,585,0,800,585,1600,292.5,790,575]"
    );

    vm.eval(
        r#"
        document.documentElement.style.cssText =
          "margin:0;height:100%;overflow:auto;scrollbar-gutter:stable both-edges";
        document.body.style.cssText = "margin:0;height:100%";
        document.body.innerHTML = '<div id="content"></div>';
        "#,
    )
    .expect("stable root gutter fixture should initialize");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify((() => {
              const html = document.documentElement.getBoundingClientRect();
              const body = document.body.getBoundingClientRect();
              return [
                document.documentElement.clientWidth,
                document.documentElement.clientHeight,
                document.documentElement.scrollWidth,
                document.documentElement.scrollHeight,
                html.x, html.width, body.x, body.width
              ];
            })())
            "#,
        )
        .expect("empty stable root gutter metrics should evaluate"),
        "[800,600,770,600,15,770,15,770]"
    );

    vm.eval("content.style.height = '1200px'")
        .expect("stable root fixture should overflow vertically");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify((() => {
              const html = document.documentElement.getBoundingClientRect();
              const body = document.body.getBoundingClientRect();
              return [
                document.documentElement.clientWidth,
                document.documentElement.clientHeight,
                document.documentElement.scrollWidth,
                document.documentElement.scrollHeight,
                html.x, html.width, body.x, body.width
              ];
            })())
            "#,
        )
        .expect("overflowing stable root gutter metrics should evaluate"),
        "[785,600,770,1200,15,770,15,770]"
    );

    vm.eval(
        r#"
        document.documentElement.style.cssText =
          "margin:0;height:100%;overflow:auto;direction:rtl";
        document.body.style.cssText = "margin:0;height:100%";
        content.style.height = "1200px";
        "#,
    )
    .expect("RTL root scrollbar fixture should initialize");
    publish_layout_for_test(&mut vm);
    let right = crate::native_bridge::element::observable_scrollbar_hit_test(
        &vm._context_host.borrow(),
        vm.document_runtime.document_handle(),
        moli_layout::LayoutPoint::new(790.0, 30.0),
    )
    .expect("RTL root right-edge hit test should succeed");
    assert!(
        right.is_some_and(|hit| {
            hit.scrollbar.axis == moli_layout::LayoutScrollbarAxis::Vertical
        })
    );
    let left = crate::native_bridge::element::observable_scrollbar_hit_test(
        &vm._context_host.borrow(),
        vm.document_runtime.document_handle(),
        moli_layout::LayoutPoint::new(5.0, 30.0),
    )
    .expect("RTL root left-edge hit test should succeed");
    assert_eq!(
        left, None,
        "the root viewport scrollbar stays physical-right"
    );
}
#[test]
fn element_scroll_methods_leave_a_detached_element_at_zero() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const div = document.createElement("div");
              div.scroll(50, 60);
              const first = [div.scrollLeft, div.scrollTop];
              div.scrollTo({ left: 10 });
              const second = [div.scrollLeft, div.scrollTop];
              div.scrollBy({ left: 5, top: 7 });
              const third = [div.scrollLeft, div.scrollTop];
              div.scroll({});
              const fourth = [div.scrollLeft, div.scrollTop];
              div.scroll();
              const fifth = [div.scrollLeft, div.scrollTop];
              return JSON.stringify({
                protoScroll: typeof Element.prototype.scroll,
                protoScrollTo: typeof Element.prototype.scrollTo,
                protoScrollBy: typeof Element.prototype.scrollBy,
                first,
                second,
                third,
                fourth,
                fifth,
                enumerable: Object.prototype.propertyIsEnumerable.call(Element.prototype, "scroll")
              });
            })()
            "#,
        )
        .expect("element scroll method probe should evaluate");

    assert_eq!(
        result,
        r#"{"protoScroll":"function","protoScrollTo":"function","protoScrollBy":"function","first":[0,0],"second":[0,0],"third":[0,0],"fourth":[0,0],"fifth":[0,0],"enumerable":false}"#
    );
}

#[test]
fn single_line_text_input_preserves_programmatic_scroll_across_select() {
    let mut vm = new_storage_test_vm("https://text-input-scroll.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          const previous = document.createElement("input");
          previous.value = "0123456789".repeat(100);
          document.body.append(previous);
          globalThis.__previousScrollInput = previous;
          return "installed";
        })()
        "#,
    )
    .expect("text input scroll fixture should initialize");
    refresh_layout_for_test(&mut vm);
    vm.eval("__previousScrollInput.scrollLeft")
        .expect("the previous input should retain the frozen layout");

    vm.eval(
        r#"
        (() => {
          __previousScrollInput.remove();
          const input = document.createElement("input");
          input.value = "0123456789".repeat(100);
          document.body.append(input);
          globalThis.__scrollInput = input;
          return "replaced";
        })()
        "#,
    )
    .expect("the previous text input should be replaced");

    refresh_layout_for_test(&mut vm);
    vm.eval("__scrollInput.scrollLeft = 33")
        .expect("text input should accept a programmatic scroll");
    refresh_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("__scrollInput.scrollWidth > __scrollInput.clientWidth")
            .expect("text input overflow should be observable"),
        "true"
    );
    assert_eq!(
        vm.eval("__scrollInput.scrollLeft")
            .expect("text input scroll should remain observable"),
        "33"
    );

    vm.eval("__scrollInput.select()")
        .expect("selecting the input contents should succeed");
    refresh_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("__scrollInput.scrollLeft")
            .expect("selection should preserve the text input scroll"),
        "33"
    );
}
