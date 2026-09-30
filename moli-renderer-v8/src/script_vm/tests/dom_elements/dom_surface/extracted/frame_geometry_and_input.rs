use super::*;

#[test]
fn element_scroll_into_view_if_needed_updates_observable_window_scroll() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }
              const visible = document.createElement("div");
              visible.id = "visible";
              document.body.appendChild(visible);

              const spacer = document.createElement("div");
              spacer.style.height = "3000px";
              document.body.appendChild(spacer);
              const target = document.createElement("div");
              target.id = "target";
              target.style.height = "20px";
              document.body.appendChild(target);
              return "installed";
            })()
            "#,
    )
    .expect("scrollIntoViewIfNeeded fixture should initialize");
    publish_layout_for_test(&mut vm);

    let result = vm
        .eval(
            r#"
            (() => {
              const visible = document.getElementById("visible");
              const target = document.getElementById("target");
              visible.scrollIntoViewIfNeeded();
              const visibleScroll = window.scrollY;
              target.scrollIntoViewIfNeeded();
              const hiddenScroll = window.scrollY;
              target.scrollIntoViewIfNeeded();
              const repeatedScroll = window.scrollY;
              window.scrollTo(0, 0);
              target.scrollIntoView({ block: "center", inline: "nearest" });

              return JSON.stringify({
                visibleScroll,
                hiddenScroll,
                repeatedScroll,
                standardScroll: window.scrollY,
                scrollingElementScroll: document.scrollingElement.scrollTop
              });
            })()
            "#,
        )
        .expect("scrollIntoViewIfNeeded probe should evaluate");

    let result: serde_json::Value =
        serde_json::from_str(&result).expect("scroll result should be JSON");
    assert_eq!(result["visibleScroll"], 0);
    assert!(
        result["hiddenScroll"]
            .as_f64()
            .is_some_and(|value| value > 0.0)
    );
    assert_eq!(result["repeatedScroll"], result["hiddenScroll"]);
    assert!(
        result["standardScroll"]
            .as_f64()
            .is_some_and(|value| value > 0.0)
    );
    assert_eq!(
        result["scrollingElementScroll"], result["standardScroll"],
        "element and Window expose the same live scroll position"
    );
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("String(document.scrollingElement.scrollTop)")
            .unwrap(),
        result["standardScroll"].to_string()
    );
}
#[test]
fn fresh_paint_publishes_nested_iframe_content_viewports() {
    let mut vm = new_storage_test_vm("https://nested-frame-publication.test/");
    vm.eval(
        r#"
        document.open();
        document.write('<!doctype html><iframe id="outer" style="width:240px;height:180px"></iframe>');
        document.close();
        const outer = document.getElementById('outer');
        const child = outer.contentDocument;
        child.open();
        child.write('<!doctype html><iframe id="inner" style="box-sizing:border-box;width:100px;height:80px;border:2px solid;padding:3px"></iframe>');
        child.close();
        "#,
    ).expect("nested frame fixture");

    let passes_before = vm.layout_pass_observability_for_test().1;
    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
        .expect("paint nested frames")
        .expect("document layout");
    assert_eq!(
        vm.eval(
            r#"JSON.stringify([
            outer.contentWindow.innerWidth, outer.contentWindow.innerHeight,
            child.getElementById('inner').contentWindow.innerWidth,
            child.getElementById('inner').contentWindow.innerHeight
        ])"#
        )
        .expect("read published frame viewports"),
        "[240,180,90,70]"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, passes_before + 1);
}
#[test]
fn transformed_constrained_iframe_routes_hover_click_and_wheel_in_child_coordinates() {
    let mut vm = new_storage_test_vm("https://iframe-input-coordinates.test/");
    vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: 1200,
        inner_height: 800,
        outer_width: 1200,
        outer_height: 800,
        device_pixel_ratio: 1.0,
        screen_width: 1200,
        screen_height: 800,
        screen_avail_width: 1200,
        screen_avail_height: 800,

        ..Default::default()
    }))
    .expect("iframe input viewport should update");

    vm.eval(
        r#"
        if (!document.documentElement) {
          document.appendChild(document.createElement('html'));
        }
        if (!document.head) {
          document.documentElement.appendChild(document.createElement('head'));
        }
        if (!document.body) {
          document.documentElement.appendChild(document.createElement('body'));
        }
        document.documentElement.style.cssText = 'margin:0;padding:0';
        document.body.style.cssText = 'margin:0;padding:0';

        const container = document.createElement('div');
        container.id = 'input-frame-clip';
        container.style.cssText = 'position:absolute;left:100px;top:120px;width:720px;height:500px;overflow:hidden';
        const frame = document.createElement('iframe');
        frame.id = 'input-frame';
        frame.style.cssText = 'display:block;width:calc(100% / 0.78);height:calc(500px / 0.78);margin:0;border:0;padding:0;transform:scale(0.78);transform-origin:0 0';
        container.appendChild(frame);
        document.body.appendChild(container);

        const child = frame.contentDocument;
        child.documentElement.style.cssText = 'margin:0;padding:0';
        child.body.style.cssText = 'margin:0;padding:0';
        const style = child.createElement('style');
        style.textContent = `
          #hover-target { position:fixed;left:220px;top:50px;width:40px;height:80px;background:red }
          #hover-target:hover { background:lime }
          #wheel-target { position:fixed;left:500px;top:180px;width:120px;height:120px;overflow:auto }
          #wheel-content { width:100px;height:900px }
        `;
        child.head.appendChild(style);
        child.body.innerHTML = `
          <div id="hover-target"></div>
          <div id="wheel-target"><div id="wheel-content"></div></div>
        `;
        frame.contentWindow.__inputEvents = [];
        for (const type of ['pointerover', 'mouseover', 'mousemove', 'mousedown', 'mouseup', 'click', 'wheel']) {
          child.addEventListener(type, event => {
            frame.contentWindow.__inputEvents.push({
              type,
              target: event.target.id,
              clientX: event.clientX,
              clientY: event.clientY,
              deltaY: event.deltaY || 0
            });
          }, true);
        }
        'installed'
        "#,
    )
    .expect("transformed iframe input fixture should initialize");

    publish_layout_for_test(&mut vm);
    let geometry = vm
        .eval(
            r#"
            (() => {
              const frame = document.getElementById('input-frame');
              const rect = frame.getBoundingClientRect();
              return JSON.stringify({
                offset: [frame.offsetWidth, frame.offsetHeight],
                rect: [rect.left, rect.top, rect.width, rect.height],
                childViewport: [frame.contentWindow.innerWidth, frame.contentWindow.innerHeight]
              });
            })()
            "#,
        )
        .expect("transformed iframe geometry should evaluate");
    let geometry: serde_json::Value =
        serde_json::from_str(&geometry).expect("iframe geometry should be JSON");
    assert_eq!(geometry["offset"], serde_json::json!([923, 641]));
    assert_eq!(geometry["childViewport"], serde_json::json!([923, 641]));
    let rect = geometry["rect"]
        .as_array()
        .expect("iframe rect should be an array");
    assert!((rect[0].as_f64().unwrap() - 100.0).abs() < 0.01);
    assert!((rect[1].as_f64().unwrap() - 120.0).abs() < 0.01);
    assert!((rect[2].as_f64().unwrap() - 719.94).abs() < 0.1);
    assert!((rect[3].as_f64().unwrap() - 499.98).abs() < 0.1);

    publish_layout_for_test(&mut vm);

    // Child point (240, 90) is the visible center of #hover-target. The
    // iframe's 0.78 transform maps it to root-frame point (287.2, 190.2).
    vm.dispatch_mouse_event_at_point(287.2, 190.2, "mousemove", -1, Some(0), 0.0, 0.0)
        .expect("child hover move should dispatch");
    vm.dispatch_mouse_event_at_point(287.2, 190.2, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("child mouse down should dispatch");
    vm.dispatch_mouse_event_at_point(287.2, 190.2, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("child mouse up should dispatch");

    // Child point (560, 240) lies in the inner overflow container.
    vm.dispatch_mouse_event_at_point(536.8, 307.2, "wheel", -1, Some(0), 0.0, 100.0)
        .expect("child wheel should dispatch");

    publish_layout_for_test(&mut vm);
    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.getElementById('input-frame');
              const child = frame.contentDocument;
              return JSON.stringify({
                hovered: child.getElementById('hover-target').matches('#hover-target:hover'),
                wheelTop: child.getElementById('wheel-target').scrollTop,
                rootTop: document.scrollingElement.scrollTop,
                events: frame.contentWindow.__inputEvents
              });
            })()
            "#,
        )
        .expect("child input result should evaluate");
    let result: serde_json::Value =
        serde_json::from_str(&result).expect("child input result should be JSON");
    assert_eq!(result["hovered"], true);
    assert_eq!(result["wheelTop"], 100);
    assert_eq!(result["rootTop"], 0);

    let events = result["events"]
        .as_array()
        .expect("child events should be an array");
    for event_type in ["mousemove", "mousedown", "mouseup", "click"] {
        let event = events
            .iter()
            .find(|event| event["type"] == event_type)
            .unwrap_or_else(|| panic!("missing {event_type} event: {events:?}"));
        assert_eq!(event["target"], "hover-target");
        assert!((event["clientX"].as_f64().unwrap() - 240.0).abs() < 0.1);
        assert!((event["clientY"].as_f64().unwrap() - 90.0).abs() < 0.1);
    }
    let wheel = events
        .iter()
        .find(|event| event["type"] == "wheel")
        .unwrap_or_else(|| panic!("missing wheel event: {events:?}"));
    assert_eq!(wheel["target"], "wheel-content");
    assert!((wheel["clientX"].as_f64().unwrap() - 560.0).abs() < 0.1);
    assert!((wheel["clientY"].as_f64().unwrap() - 240.0).abs() < 0.1);

    vm.eval("document.getElementById('input-frame-clip').style.width='624px'")
        .unwrap();
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const frame = document.getElementById('input-frame');
              return [frame.offsetWidth, frame.contentWindow.innerWidth].join('|');
            })()
            "#,
        )
        .expect("resized iframe viewport should evaluate"),
        "800|800"
    );
}
#[test]
fn iframe_input_reuses_one_top_level_snapshot_without_parent_child_ping_pong() {
    let mut vm = new_storage_test_vm("https://iframe-input-snapshot.test/");
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
    .expect("iframe input snapshot viewport should update");
    vm.eval(
        r#"
        if (!document.documentElement) {
          document.appendChild(document.createElement('html'));
        }
        if (!document.body) {
          document.documentElement.appendChild(document.createElement('body'));
        }
        document.documentElement.style.cssText = 'margin:0;padding:0';
        document.body.style.cssText = 'margin:0;padding:0';
        const frame = document.createElement('iframe');
        frame.id = 'snapshot-frame';
        frame.style.cssText = 'position:absolute;left:100px;top:80px;width:240px;height:180px;border:0;padding:0';
        document.body.appendChild(frame);
        const child = frame.contentDocument;
        child.documentElement.style.cssText = 'margin:0;padding:0';
        child.body.style.cssText = 'margin:0;padding:0';
        child.body.innerHTML = '<button id="snapshot-target" style="position:fixed;left:20px;top:20px;width:80px;height:60px">target</button>';
        'installed'
        "#,
    )
    .expect("iframe input snapshot fixture should initialize");

    let passes_before = vm.layout_pass_observability_for_test().1;
    vm.dispatch_mouse_event_at_point(140.0, 130.0, "mousemove", -1, Some(0), 0.0, 0.0)
        .expect("cold input should publish the parent and iframe together");
    let prepared = vm.layout_pass_observability_for_test().1;
    assert_eq!(prepared, passes_before + 1);
    assert_eq!(vm.eval("document.body.offsetWidth > 0").unwrap(), "true");
    for _ in 0..3 {
        vm.dispatch_mouse_event_at_point(140.0, 130.0, "mousemove", -1, Some(0), 0.0, 0.0)
            .expect("child hover should consume the published composite snapshot");
        assert_eq!(vm.layout_pass_observability_for_test().1, prepared);
    }

    let cached = vm
        .layout_snapshot_cache_observability_for_test()
        .3
        .expect("stable child input should retain one snapshot");
    assert_eq!(cached.0, vm.document_runtime.document_handle());
}
#[test]
fn iframe_wheel_batch_reuses_one_composite_snapshot_for_every_scroll_step() {
    let mut vm = new_storage_test_vm("https://iframe-wheel-snapshot.test/");
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
    .expect("iframe wheel snapshot viewport should update");
    vm.eval(
        r#"
        if (!document.documentElement) {
          document.appendChild(document.createElement('html'));
        }
        if (!document.body) {
          document.documentElement.appendChild(document.createElement('body'));
        }
        document.documentElement.style.cssText = 'margin:0;padding:0';
        document.body.style.cssText = 'margin:0;padding:0';
        const frame = document.createElement('iframe');
        frame.id = 'wheel-frame';
        frame.style.cssText = 'position:absolute;left:100px;top:80px;width:240px;height:180px;border:0;padding:0';
        document.body.appendChild(frame);
        const child = frame.contentDocument;
        child.documentElement.style.cssText = 'margin:0;padding:0';
        child.body.style.cssText = 'margin:0;padding:0';
        child.body.innerHTML = `
          <div id="wheel-scroller" style="position:absolute;left:10px;top:10px;width:160px;height:100px;overflow:auto">
            <div id="wheel-content" style="width:140px;height:500px"></div>
          </div>`;
        child.defaultView.__wheelGeometryReads = 0;
        child.getElementById('wheel-scroller').addEventListener('wheel', event => {
          void event.currentTarget.offsetWidth;
          child.defaultView.__wheelGeometryReads++;
        });
        'installed'
        "#,
    )
    .expect("iframe wheel snapshot fixture should initialize");

    publish_layout_for_test(&mut vm);
    let passes_before = vm.layout_pass_observability_for_test().1;
    vm.begin_batched_mouse_event_dispatch();
    for delta_y in [10.0, 20.0, 30.0] {
        vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers_without_checkpoint(
            130.0,
            110.0,
            "wheel",
            -1,
            Some(0),
            0,
            0.0,
            delta_y,
            crate::runtime::RendererPointerEventProperties::default(),
            0,
        )
        .expect("batched child wheel step should dispatch");
    }
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        passes_before,
        "every wheel step must share the previously published snapshot"
    );
    vm.finish_batched_mouse_event_dispatch(Ok(()), true)
        .expect("batched child wheel effects should commit");
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        passes_before,
        "committing derived effects must not perform another layout"
    );

    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval(
            "(() => { const frame = document.getElementById('wheel-frame'); return [frame.contentDocument.getElementById('wheel-scroller').scrollTop, frame.contentWindow.__wheelGeometryReads].join('|'); })()"
        )
        .expect("child scroll position should evaluate"),
        "60|3"
    );
}
#[test]
fn focusing_visible_child_target_does_not_scroll_partially_hidden_transformed_iframe() {
    let mut vm = new_storage_test_vm("https://iframe-focus-scroll.test/");
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
    .expect("iframe focus viewport should update");

    vm.eval(
        r#"
        if (!document.documentElement) {
          document.appendChild(document.createElement('html'));
        }
        if (!document.body) {
          document.documentElement.appendChild(document.createElement('body'));
        }
        document.documentElement.style.cssText = 'margin:0;padding:0';
        document.body.style.cssText = 'margin:0;padding:0;min-height:1200px';

        const clip = document.createElement('div');
        clip.style.cssText = 'position:absolute;left:100px;top:400px;width:720px;height:500px;overflow:hidden';
        const frame = document.createElement('iframe');
        frame.id = 'focus-frame';
        frame.style.cssText = 'display:block;width:calc(100% / .78);height:calc(500px / .78);margin:0;border:0;padding:0;transform:scale(.78);transform-origin:0 0';
        clip.appendChild(frame);
        document.body.appendChild(clip);

        const child = frame.contentDocument;
        child.documentElement.style.cssText = 'margin:0;padding:0';
        child.body.style.cssText = 'margin:0;padding:0';
        child.body.innerHTML = '<button id="focus-target" style="position:fixed;left:20px;top:20px;width:80px;height:40px">Run</button>';
        frame.contentWindow.__focusEvents = [];
        for (const type of ['mousemove', 'mousedown', 'focus', 'mouseup', 'click']) {
          child.addEventListener(type, event => {
            frame.contentWindow.__focusEvents.push({
              type,
              target: event.target.id,
              clientX: event.clientX || 0,
              clientY: event.clientY || 0
            });
          }, true);
        }
        'installed'
        "#,
    )
    .expect("iframe focus fixture should initialize");

    publish_layout_for_test(&mut vm);

    // Child point (60, 40), the button center, maps through scale(.78) to
    // root-frame point (146.8, 431.2). The button is visible, while the frame
    // continues below the 600px-high top viewport.
    vm.dispatch_mouse_event_at_point(146.8, 431.2, "mousemove", -1, Some(0), 0.0, 0.0)
        .expect("child hover should dispatch");
    vm.dispatch_mouse_event_at_point(146.8, 431.2, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("child press should dispatch");

    assert_eq!(
        vm.eval("String(window.scrollY)")
            .expect("parent scroll after child focus should evaluate"),
        "0",
        "focusing a visible child target must not reveal the whole iframe"
    );

    vm.dispatch_mouse_event_at_point(146.8, 431.2, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("child release should dispatch");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.getElementById('focus-frame');
              const child = frame.contentDocument;
              return JSON.stringify({
                parentScroll: window.scrollY,
                parentActive: document.activeElement === frame,
                childActive: child.activeElement === child.getElementById('focus-target'),
                hovered: child.getElementById('focus-target').matches('#focus-target:hover'),
                events: frame.contentWindow.__focusEvents
              });
            })()
            "#,
        )
        .expect("iframe focus result should evaluate");
    let result: serde_json::Value =
        serde_json::from_str(&result).expect("iframe focus result should be JSON");
    assert_eq!(result["parentScroll"], 0);
    assert_eq!(result["parentActive"], true);
    assert_eq!(result["childActive"], true);
    assert_eq!(result["hovered"], true);

    let events = result["events"]
        .as_array()
        .expect("iframe focus events should be an array");
    for event_type in ["mousemove", "mousedown", "mouseup", "click"] {
        let event = events
            .iter()
            .find(|event| event["type"] == event_type)
            .unwrap_or_else(|| panic!("missing child {event_type}: {events:?}"));
        assert_eq!(event["target"], "focus-target");
        assert!((event["clientX"].as_f64().unwrap() - 60.0).abs() < 0.1);
        assert!((event["clientY"].as_f64().unwrap() - 40.0).abs() < 0.1);
    }
}
#[test]
fn nested_transformed_iframe_input_composes_scroll_border_padding_and_exit_coordinates() {
    let mut vm = new_storage_test_vm("https://nested-iframe-input.test/");
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
    .expect("nested iframe viewport should update");

    vm.eval(
        r#"
        if (!document.documentElement) {
          document.appendChild(document.createElement('html'));
        }
        if (!document.body) {
          document.documentElement.appendChild(document.createElement('body'));
        }
        document.documentElement.style.cssText = 'margin:0;padding:0';
        document.body.style.cssText = 'margin:0;padding:0';

        const outside = document.createElement('div');
        outside.id = 'outside';
        outside.style.cssText = 'position:fixed;left:0;top:0;width:30px;height:30px';
        document.body.appendChild(outside);

        const parentScroller = document.createElement('div');
        parentScroller.id = 'parent-scroller';
        parentScroller.style.cssText = 'position:absolute;left:40px;top:30px;width:400px;height:300px;overflow:auto';
        const canvas = document.createElement('div');
        canvas.style.cssText = 'position:relative;width:800px;height:600px';
        const outerFrame = document.createElement('iframe');
        outerFrame.id = 'outer-frame';
        outerFrame.style.cssText = 'position:absolute;left:100px;top:80px;display:block;box-sizing:border-box;width:240px;height:180px;margin:0;border:4px solid black;padding:6px;transform:scale(.75);transform-origin:0 0';
        canvas.appendChild(outerFrame);
        parentScroller.appendChild(canvas);
        document.body.appendChild(parentScroller);

        const child = outerFrame.contentDocument;
        child.documentElement.style.cssText = 'margin:0;padding:0';
        child.body.style.cssText = 'margin:0;padding:0';
        const nestedFrame = child.createElement('iframe');
        nestedFrame.id = 'nested-frame';
        nestedFrame.style.cssText = 'position:absolute;left:40px;top:30px;display:block;box-sizing:border-box;width:100px;height:80px;margin:0;border:2px solid black;padding:3px;transform:scale(.5);transform-origin:0 0';
        child.body.appendChild(nestedFrame);

        const nested = nestedFrame.contentDocument;
        nested.documentElement.style.cssText = 'margin:0;padding:0';
        nested.body.style.cssText = 'margin:0;padding:0';
        nested.body.innerHTML = `
          <div id="nested-target" style="position:fixed;left:10px;top:10px;width:20px;height:20px"></div>
          <div id="nested-scroll" style="position:fixed;left:10px;top:30px;width:50px;height:40px;overflow:auto">
            <div id="nested-scroll-content" style="width:200px;height:200px"></div>
          </div>
        `;
        nestedFrame.contentWindow.__nestedEvents = [];
        for (const type of ['pointerover', 'pointerout', 'mouseover', 'mouseout', 'mousemove', 'mousedown', 'mouseup', 'click', 'wheel']) {
          nested.addEventListener(type, event => {
            nestedFrame.contentWindow.__nestedEvents.push({
              type,
              target: event.target.id,
              clientX: event.clientX,
              clientY: event.clientY
            });
          }, true);
        }
        'installed'
        "#,
    )
    .expect("nested transformed iframe fixture should initialize");

    publish_layout_for_test(&mut vm);
    vm.eval("parentScroller.scrollTo(50, 40)").unwrap();
    publish_layout_for_test(&mut vm);
    // The exact used content viewports exclude each frame's border and
    // padding: 240 - 2*(4+6) = 220, then 100 - 2*(2+3) = 90.
    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const outer = document.getElementById('outer-frame');
              const nested = outer.contentDocument.getElementById('nested-frame');
              return JSON.stringify([
                outer.clientWidth - 12, outer.clientHeight - 12,
                nested.clientWidth - 6, nested.clientHeight - 6,
                document.getElementById('parent-scroller').scrollLeft,
                document.getElementById('parent-scroller').scrollTop
              ]);
            })()
            "#,
        )
        .expect("nested frame geometry should evaluate"),
        "[220,160,90,70,50,40]"
    );

    publish_layout_for_test(&mut vm);

    // Nested client point (20, 20) maps through scale(.5), the inner frame's
    // 5px border+padding edge, scale(.75), the outer frame's 10px edge, and
    // the scrolled parent to root point (136.875, 109.375).
    for event_name in ["mousemove", "mousedown", "mouseup"] {
        let (button, buttons) = match event_name {
            "mousedown" => (0, Some(1)),
            "mouseup" => (0, Some(0)),
            _ => (-1, Some(0)),
        };
        vm.dispatch_mouse_event_at_point(136.875, 109.375, event_name, button, buttons, 0.0, 0.0)
            .unwrap_or_else(|error| panic!("nested {event_name} should dispatch: {error:#}"));
    }

    // Nested client point (20, 35) is inside the scroll content.
    vm.dispatch_mouse_event_at_point(136.875, 115.0, "wheel", -1, Some(0), 0.0, 30.0)
        .expect("nested wheel should dispatch");
    let events_before_scrollbar = vm
        .eval(
            "String(document.getElementById('outer-frame').contentDocument.getElementById('nested-frame').contentWindow.__nestedEvents.length)",
        )
        .expect("nested event count should evaluate")
        .parse::<usize>()
        .expect("nested event count should be numeric");
    // Nested client point (52, 50) is the vertical scrollbar's forward
    // button. UA scrollbar input is routed through the same two frame maps and
    // remains outside DOM mouse dispatch.
    vm.dispatch_mouse_event_at_point(148.875, 120.625, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("nested scrollbar press should dispatch");
    vm.dispatch_mouse_event_at_point(148.875, 120.625, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("nested scrollbar release should dispatch");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const nestedFrame = document.getElementById('outer-frame').contentDocument.getElementById('nested-frame');
              return JSON.stringify([
                nestedFrame.contentDocument.getElementById('nested-scroll').scrollTop,
                nestedFrame.contentWindow.__nestedEvents.length
              ]);
            })()
            "#,
        )
        .expect("nested scrollbar result should evaluate"),
        format!("[70,{events_before_scrollbar}]")
    );
    // Move to a top-document element. The outgoing events must still convert
    // this new root point through the previous nested frame chain.
    vm.dispatch_mouse_event_at_point(10.0, 10.0, "mousemove", -1, Some(0), 0.0, 0.0)
        .expect("nested hover exit should dispatch");

    let result = vm
        .eval(
            r#"
            (() => {
              const outer = document.getElementById('outer-frame');
              const nestedFrame = outer.contentDocument.getElementById('nested-frame');
              const nested = nestedFrame.contentDocument;
              return JSON.stringify({
                events: nestedFrame.contentWindow.__nestedEvents,
                scrollTop: nested.getElementById('nested-scroll').scrollTop,
                parentTop: document.getElementById('parent-scroller').scrollTop
              });
            })()
            "#,
        )
        .expect("nested iframe input result should evaluate");
    let result: serde_json::Value =
        serde_json::from_str(&result).expect("nested iframe result should be JSON");
    assert_eq!(result["scrollTop"], 70);
    assert_eq!(result["parentTop"], 40);
    let events = result["events"]
        .as_array()
        .expect("nested events should be an array");
    for event_type in ["mousemove", "mousedown", "mouseup", "click"] {
        let event = events
            .iter()
            .find(|event| event["type"] == event_type && event["target"] == "nested-target")
            .unwrap_or_else(|| panic!("missing nested {event_type}: {events:?}"));
        assert!((event["clientX"].as_f64().unwrap() - 20.0).abs() < 0.1);
        assert!((event["clientY"].as_f64().unwrap() - 20.0).abs() < 0.1);
    }
    let wheel = events
        .iter()
        .find(|event| event["type"] == "wheel")
        .unwrap_or_else(|| panic!("missing nested wheel: {events:?}"));
    assert_eq!(wheel["target"], "nested-scroll-content");
    assert!((wheel["clientX"].as_f64().unwrap() - 20.0).abs() < 0.1);
    assert!((wheel["clientY"].as_f64().unwrap() - 35.0).abs() < 0.1);
    let mouseout = events
        .iter()
        .rev()
        .find(|event| event["type"] == "mouseout")
        .unwrap_or_else(|| panic!("missing nested mouseout: {events:?}"));
    assert_eq!(mouseout["target"], "nested-target");
    assert!((mouseout["clientX"].as_f64().unwrap() + 318.333).abs() < 0.2);
    assert!((mouseout["clientY"].as_f64().unwrap() + 245.0).abs() < 0.2);
}
#[test]
fn root_classic_scrollbars_stay_viewport_fixed_and_drive_window_scroll() {
    let mut vm = new_storage_test_vm("https://root-classic-scrollbar.test/");
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
    .expect("root scrollbar viewport surface should update");
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
          document.body.innerHTML = '<div style="width:1600px;height:1200px"></div>';
          window.__rootScrollbarDomEvents = [];
          for (const name of ["pointerdown", "pointermove", "pointerup", "mousedown", "mousemove", "mouseup", "click"]) {
            document.addEventListener(name, () => __rootScrollbarDomEvents.push(name), true);
          }
        })()
        "#,
    )
    .expect("root scrollbar fixture should initialize");
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval(
            "JSON.stringify([document.documentElement.clientWidth, document.documentElement.clientHeight, document.documentElement.scrollWidth, document.documentElement.scrollHeight])"
        )
        .expect("root scrollbar metrics should evaluate"),
        "[785,585,1600,1200]"
    );

    let horizontal_hit = crate::native_bridge::element::observable_scrollbar_hit_test(
        &vm._context_host.borrow(),
        vm.document_runtime.document_handle(),
        moli_layout::LayoutPoint::new(30.0, 590.0),
    )
    .expect("root horizontal scrollbar hit test should succeed");
    assert!(
        horizontal_hit.is_some_and(|hit| {
            hit.scrollbar.axis == moli_layout::LayoutScrollbarAxis::Horizontal
                && hit.part == moli_layout::LayoutScrollbarPart::Thumb
        }),
        "root horizontal thumb should own its viewport-fixed coordinates: {horizontal_hit:?}"
    );

    vm.dispatch_mouse_event_at_point(790.0, 30.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("root vertical thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(790.0, 100.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("root vertical thumb drag should dispatch");
    vm.dispatch_mouse_event_at_point(790.0, 100.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("root vertical thumb release should dispatch");
    let horizontal_hit_after_vertical_drag =
        crate::native_bridge::element::observable_scrollbar_hit_test(
            &vm._context_host.borrow(),
            vm.document_runtime.document_handle(),
            moli_layout::LayoutPoint::new(30.0, 590.0),
        )
        .expect("post-scroll root horizontal scrollbar hit test should succeed");
    let root_extent_after_vertical_drag = vm
        ._context_host
        .borrow()
        .with_latest_layout_tree_for_document(vm.document_runtime.document_handle(), |tree| {
            tree.scroll_extent(tree.root_box).cloned()
        })
        .flatten();
    assert!(
        horizontal_hit_after_vertical_drag.is_some_and(|hit| {
            hit.scrollbar.axis == moli_layout::LayoutScrollbarAxis::Horizontal
                && hit.part == moli_layout::LayoutScrollbarPart::Thumb
        }),
        "root horizontal thumb should stay viewport-fixed after vertical scrolling: hit={horizontal_hit_after_vertical_drag:?}, extent={root_extent_after_vertical_drag:?}"
    );
    vm.dispatch_mouse_event_at_point(30.0, 590.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("root horizontal thumb press should dispatch");
    vm.dispatch_mouse_event_at_point(100.0, 590.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("root horizontal thumb drag should dispatch");
    vm.dispatch_mouse_event_at_point(100.0, 590.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("root horizontal thumb release should dispatch");

    let result = vm
        .eval("JSON.stringify([window.scrollX, window.scrollY, __rootScrollbarDomEvents])")
        .expect("root post-drag state should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).expect("root scroll JSON");
    assert!(
        result[0].as_f64().is_some_and(|value| value > 100.0),
        "root horizontal thumb drag should advance scrollX: {result}"
    );
    assert!(
        result[1].as_f64().is_some_and(|value| value > 100.0),
        "root vertical thumb drag should advance scrollY: {result}"
    );
    assert_eq!(result[2], serde_json::json!([]));
}
