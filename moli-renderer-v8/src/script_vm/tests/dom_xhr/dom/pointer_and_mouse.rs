use super::*;

#[test]
fn mouse_drop_requires_prevented_dragover() {
    let mut vm = new_rendered_test_vm(
        "https://drop-requires-prevented-dragover.test/",
        r#"<html><body><div id="drag" draggable="true">drag</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  window.__allowDrop = false;
  window.__dragLog = [];
  window.__dropCount = 0;
  const drag = document.getElementById('drag');
  drag.addEventListener('dragstart', () => {
    window.__dragLog.push('dragstart');
  });
  drag.addEventListener('dragover', event => {
    window.__dragLog.push(`dragover:${window.__allowDrop}`);
    if (window.__allowDrop) {
      event.preventDefault();
    }
  });
  drag.addEventListener('drop', () => {
    window.__dropCount += 1;
    window.__dragLog.push('drop');
  });
})()
"#,
    )
    .expect("drag listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("first mousedown should dispatch");
    vm.dispatch_mouse_event_at_point(30.0, 20.0, "mousemove", 0, Some(1), 0.0, 0.0)
        .expect("first mousemove should start drag");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("first mouseup should finish drag");

    let first_result = vm
        .eval(
            r#"
(() => [window.__dropCount, window.__dragLog.join('|')].join('|'))()
"#,
        )
        .expect("first drag log should evaluate");
    assert_eq!(first_result, "0|dragstart|dragover:false");

    vm.eval("window.__allowDrop = true")
        .expect("drop permission flag should update");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("second mousedown should dispatch");
    vm.dispatch_mouse_event_at_point(30.0, 20.0, "mousemove", 0, Some(1), 0.0, 0.0)
        .expect("second mousemove should start drag");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("second mouseup should finish drag");

    let second_result = vm
        .eval(
            r#"
(() => [window.__dropCount, window.__dragLog.join('|')].join('|'))()
"#,
        )
        .expect("second drag log should evaluate");
    assert_eq!(
        second_result,
        "1|dragstart|dragover:false|dragstart|dragover:true|drop"
    );
}

#[test]
fn mouse_dragstart_prevent_default_cancels_drag_session() {
    let mut vm = new_rendered_test_vm(
        "https://dragstart-prevent-default-cancels.test/",
        r#"<html><body><div id="drag" draggable="true">drag</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  window.__dragStarts = 0;
  window.__dragOvers = 0;
  window.__drops = 0;
  const drag = document.getElementById('drag');
  drag.addEventListener('dragstart', event => {
    window.__dragStarts += 1;
    event.preventDefault();
  });
  drag.addEventListener('dragover', event => {
    window.__dragOvers += 1;
    event.preventDefault();
  });
  drag.addEventListener('drop', () => {
    window.__drops += 1;
  });
})()
"#,
    )
    .expect("drag cancel listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch");
    vm.dispatch_mouse_event_at_point(30.0, 20.0, "mousemove", 0, Some(1), 0.0, 0.0)
        .expect("mousemove should attempt drag");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("mouseup should dispatch");

    let result = vm
        .eval(
            r#"
(() => [window.__dragStarts, window.__dragOvers, window.__drops].join('|'))()
"#,
        )
        .expect("drag cancel result should evaluate");
    assert_eq!(result, "1|0|0");
}

#[test]
fn mouse_dispatch_emits_pointer_event_properties() {
    let mut vm = new_rendered_test_vm(
        "https://pointer-event-properties.test/",
        r#"<html><body><button id="target">tap</button></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  window.__pointerLog = [];
  for (const type of ['pointerdown', 'pointerup']) {
    window.addEventListener(type, event => {
      window.__pointerLog.push([
        event.type,
        event.pointerType,
        event.pressure,
        event.tangentialPressure,
        event.tiltX,
        event.tiltY,
        event.twist,
        event.clientX,
        event.clientY,
        event.button,
        event.buttons
      ].join(':'));
    });
  }
})()
"#,
    )
    .expect("pointer listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point_with_pointer(
        20.0,
        20.0,
        "mousedown",
        0,
        None,
        0,
        0.0,
        0.0,
        crate::runtime::RendererPointerEventProperties {
            pointer_id: 1,
            pointer_type: "pen".to_owned(),
            pressure: 0.75,
            tangential_pressure: -0.25,
            tilt_x: 12.0,
            tilt_y: -8.0,
            twist: 45.0,
        },
    )
    .expect("mousedown should dispatch pointerdown");
    vm.dispatch_mouse_event_at_point_with_pointer(
        20.0,
        20.0,
        "mouseup",
        0,
        None,
        0,
        0.0,
        0.0,
        crate::runtime::RendererPointerEventProperties {
            pointer_id: 1,
            pointer_type: "pen".to_owned(),
            pressure: 0.0,
            tangential_pressure: 0.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            twist: 0.0,
        },
    )
    .expect("mouseup should dispatch pointerup");

    let result = vm
        .eval("window.__pointerLog.join('|')")
        .expect("pointer log should evaluate");
    assert_eq!(
        result,
        "pointerdown:pen:0.75:-0.25:12:-8:45:20:20:0:1|pointerup:pen:0:0:0:0:0:20:20:0:0"
    );
}

#[test]
fn canceled_pointerdown_suppresses_compat_mouse_events_but_keeps_click() {
    let mut vm = new_rendered_test_vm(
        "https://pointerdown-suppresses-compat-mouse.test/",
        r#"<html><body><button id="target">tap</button></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target = document.getElementById('target');
  window.__compatLog = [];
  for (const type of ['pointerdown', 'pointerup', 'mousedown', 'mouseup', 'click']) {
    target.addEventListener(type, event => {
      window.__compatLog.push(event.type);
      if (event.type === 'pointerdown') {
        event.preventDefault();
      }
    });
  }
})()
"#,
    )
    .expect("compat suppression listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch pointerdown");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("mouseup should dispatch pointerup and click");

    let result = vm
        .eval("window.__compatLog.join('|')")
        .expect("compat log should evaluate");
    assert_eq!(result, "pointerdown|pointerup|click");
}

#[test]
fn pointer_capture_routes_mouse_pointer_until_pointerup() {
    let mut vm = new_rendered_test_vm(
        "https://pointer-capture-routes-mouse.test/",
        r#"<html><body><div id="target0">first</div><div id="target1">second</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target0 = document.getElementById('target0');
  const target1 = document.getElementById('target1');
  window.__captureStarted = false;
  window.__captureLog = [];
  for (const target of [target0, target1]) {
    for (const type of ['pointerdown', 'gotpointercapture', 'pointermove', 'pointerup', 'lostpointercapture']) {
      target.addEventListener(type, event => {
        if (event.type === 'pointermove' && !window.__captureStarted) {
          return;
        }
        window.__captureLog.push(`${event.type}@${target.id}`);
        if (event.type === 'pointermove' && target === target0) {
          window.__captureLog.push(`activeHas:${target0.hasPointerCapture(event.pointerId)}`);
        }
        if (event.type === 'pointerdown' && target === target0) {
          window.__captureStarted = true;
          target0.setPointerCapture(event.pointerId);
          window.__captureLog.push(`has:${target0.hasPointerCapture(event.pointerId)}`);
        }
      });
    }
  }
})()
"#,
    )
    .expect("pointer capture listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch pointerdown");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mousemove", -1, None, 0.0, 0.0)
        .expect("captured pointermove should dispatch to capture target");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("captured pointerup should release capture");

    let result = vm
        .eval("window.__captureLog.join('|')")
        .expect("capture log should evaluate");
    assert_eq!(
        result,
        "pointerdown@target0|has:true|gotpointercapture@target0|pointermove@target0|activeHas:true|pointerup@target0|lostpointercapture@target0"
    );
}

#[test]
fn pointer_capture_lost_dispatches_before_compat_mouseup() {
    let mut vm = new_rendered_test_vm(
        "https://pointer-capture-lost-before-mouseup.test/",
        r#"<html><body><div id="target0">capture</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target0 = document.getElementById('target0');
  window.__captureMouseupOrder = [];
  for (const type of ['pointerdown', 'gotpointercapture', 'pointerup', 'lostpointercapture', 'mouseup']) {
    target0.addEventListener(type, event => {
      window.__captureMouseupOrder.push(event.type);
      if (event.type === 'pointerdown') {
        target0.setPointerCapture(event.pointerId);
      }
    });
  }
})()
"#,
    )
    .expect("pointer capture mouseup order listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch pointerdown");
    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("mouseup should dispatch pointerup, lostpointercapture, mouseup");

    let result = vm
        .eval("window.__captureMouseupOrder.join('|')")
        .expect("capture mouseup order log should evaluate");
    assert_eq!(
        result,
        "pointerdown|gotpointercapture|pointerup|lostpointercapture|mouseup"
    );
}

#[test]
fn pointer_capture_mouse_events_preserve_modifiers() {
    let mut vm = new_rendered_test_vm(
        "https://pointer-capture-mouse-modifiers.test/",
        r#"<html><body><div id="target0">capture</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target0 = document.getElementById('target0');
  window.__captureModifierLog = [];
  for (const type of ['pointerdown', 'gotpointercapture', 'pointerup', 'lostpointercapture']) {
    target0.addEventListener(type, event => {
      window.__captureModifierLog.push(`${event.type}:${event.ctrlKey}:${event.shiftKey}:${event.altKey}:${event.metaKey}`);
      if (event.type === 'pointerdown') {
        target0.setPointerCapture(event.pointerId);
      }
    });
  }
})()
"#,
    )
    .expect("pointer capture modifier listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
        10.0,
        11.0,
        "mousedown",
        0,
        None,
        0,
        0.0,
        0.0,
        crate::runtime::RendererPointerEventProperties::default(),
        10,
    )
    .expect("mousedown should dispatch pointerdown with modifiers");
    vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
        10.0,
        11.0,
        "mouseup",
        0,
        None,
        0,
        0.0,
        0.0,
        crate::runtime::RendererPointerEventProperties::default(),
        10,
    )
    .expect("mouseup should dispatch pointerup and capture release with modifiers");

    let result = vm
        .eval("window.__captureModifierLog.join('|')")
        .expect("capture modifier log should evaluate");
    assert_eq!(
        result,
        "pointerdown:true:true:false:false|gotpointercapture:true:true:false:false|pointerup:true:true:false:false|lostpointercapture:true:true:false:false"
    );
}

#[test]
fn touch_pointer_implicit_capture_routes_until_pointerup() {
    let mut vm = new_rendered_test_vm(
        "https://touch-pointer-implicit-capture.test/",
        r#"<html><body><div id="target0">first</div><div id="target1">second</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target0 = document.getElementById('target0');
  const target1 = document.getElementById('target1');
  window.__touchImplicitCaptureLog = [];
  for (const target of [target0, target1]) {
    for (const type of ['pointerdown', 'gotpointercapture', 'pointermove', 'pointerup', 'lostpointercapture']) {
      target.addEventListener(type, event => {
        window.__touchImplicitCaptureLog.push(`${event.type}@${target.id}:${event.pointerType}`);
        if (event.type === 'pointerdown' && target === target0) {
          window.__touchImplicitCaptureLog.push(`has:${target0.hasPointerCapture(event.pointerId)}`);
        }
      });
    }
  }
})()
"#,
    )
    .expect("touch implicit capture listener setup should evaluate");

    vm.dispatch_touch_event_at_point(10.0, 11.0, "touchstart", false)
        .expect("touchstart should dispatch pointerdown with implicit capture");
    vm.dispatch_touch_event_at_point(10.0, 35.0, "touchmove", false)
        .expect("touchmove should route to implicit capture target");
    vm.dispatch_touch_event_at_point(10.0, 35.0, "touchend", false)
        .expect("touchend should release implicit capture");

    let result = vm
        .eval("window.__touchImplicitCaptureLog.join('|')")
        .expect("touch implicit capture log should evaluate");
    assert_eq!(
        result,
        "pointerdown@target0:touch|has:true|gotpointercapture@target0:touch|pointermove@target0:touch|pointerup@target0:touch|lostpointercapture@target0:touch"
    );
}

#[test]
fn touch_pointer_capture_lost_dispatches_before_touchend() {
    let mut vm = new_rendered_test_vm(
        "https://touch-pointer-lost-before-touchend.test/",
        r#"<html><body><div id="target0">capture</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target0 = document.getElementById('target0');
  window.__touchEndOrder = [];
  for (const type of ['pointerdown', 'gotpointercapture', 'pointerup', 'lostpointercapture', 'touchend']) {
    target0.addEventListener(type, event => {
      window.__touchEndOrder.push(event.type);
    });
  }
})()
"#,
    )
    .expect("touch capture touchend order listener setup should evaluate");

    vm.dispatch_touch_event_at_point(10.0, 11.0, "touchstart", false)
        .expect("touchstart should dispatch pointerdown");
    vm.dispatch_touch_event_at_point(10.0, 11.0, "touchend", false)
        .expect("touchend should dispatch pointerup, lostpointercapture, touchend");

    let result = vm
        .eval("window.__touchEndOrder.join('|')")
        .expect("touchend order log should evaluate");
    assert_eq!(
        result,
        "pointerdown|gotpointercapture|pointerup|lostpointercapture|touchend"
    );
}

#[test]
fn touch_pointer_capture_can_route_to_explicit_capture_target() {
    let mut vm = new_rendered_test_vm(
        "https://touch-pointer-explicit-capture.test/",
        r#"<html><body><div id="button">button</div><div id="target0">capture</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const button = document.getElementById('button');
  const target0 = document.getElementById('target0');
  window.__touchExplicitCaptureLog = [];
  button.addEventListener('pointerdown', event => {
    window.__touchExplicitCaptureLog.push(`pointerdown@button:${event.pointerType}`);
    target0.setPointerCapture(event.pointerId);
    window.__touchExplicitCaptureLog.push(`has:${target0.hasPointerCapture(event.pointerId)}`);
  });
  button.addEventListener('pointermove', () => {
    window.__touchExplicitCaptureLog.push('pointermove@button');
  });
  target0.addEventListener('gotpointercapture', event => {
    window.__touchExplicitCaptureLog.push(`gotpointercapture@target0:${event.pointerType}`);
  });
  target0.addEventListener('pointermove', event => {
    window.__touchExplicitCaptureLog.push(`pointermove@target0:${event.pointerType}`);
  });
  target0.addEventListener('pointerup', event => {
    window.__touchExplicitCaptureLog.push(`pointerup@target0:${event.pointerType}`);
  });
  target0.addEventListener('lostpointercapture', event => {
    window.__touchExplicitCaptureLog.push(`lostpointercapture@target0:${event.pointerType}`);
  });
})()
"#,
    )
    .expect("touch explicit capture listener setup should evaluate");

    vm.dispatch_touch_event_at_point(10.0, 11.0, "touchstart", false)
        .expect("touchstart should dispatch pointerdown");
    vm.dispatch_touch_event_at_point(10.0, 35.0, "touchmove", false)
        .expect("touchmove should dispatch to explicit capture target");
    vm.dispatch_touch_event_at_point(10.0, 35.0, "touchend", false)
        .expect("touchend should release explicit capture");

    let result = vm
        .eval("window.__touchExplicitCaptureLog.join('|')")
        .expect("touch explicit capture log should evaluate");
    assert_eq!(
        result,
        "pointerdown@button:touch|has:true|gotpointercapture@target0:touch|pointermove@target0:touch|pointerup@target0:touch|lostpointercapture@target0:touch"
    );
}

#[test]
fn pointer_raw_update_dispatches_after_pointer_boundary_before_mouse_boundary() {
    let mut vm = new_rendered_test_vm(
        "https://pointer-raw-update-order.test/",
        r#"<html><body><div id="init">init</div><div id="target">target</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target = document.getElementById('target');
  window.__rawUpdateOrder = [`exposed:${'onpointerrawupdate' in target}`];
  function log(event) {
    window.__rawUpdateOrder.push(event.type);
  }
  for (const type of ['pointerover', 'pointerenter', 'pointerrawupdate', 'pointermove', 'mouseover', 'mouseenter']) {
    target.addEventListener(type, log);
  }
  target.addEventListener('pointerrawupdate', () => {
    target.removeEventListener('mouseover', log);
    target.removeEventListener('mouseenter', log);
  }, { once: true });
})()
"#,
    )
    .expect("pointer raw update listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousemove", -1, None, 0.0, 0.0)
        .expect("initial mousemove should establish previous hover target");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mousemove", -1, None, 0.0, 0.0)
        .expect("second mousemove should dispatch raw update before mouse boundary");

    let result = vm
        .eval("window.__rawUpdateOrder.join('|')")
        .expect("raw update order log should evaluate");
    assert_eq!(
        result,
        "exposed:true|pointerover|pointerenter|pointerrawupdate|pointermove"
    );
}

#[test]
fn pointer_raw_update_flushes_capture_before_pointermove() {
    let mut vm = new_rendered_test_vm(
        "https://pointer-raw-update-capture.test/",
        r#"<html><body><div id="target0">first</div><div id="target1">second</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target0 = document.getElementById('target0');
  const target1 = document.getElementById('target1');
  window.__rawUpdateCaptureLog = [];
  target0.addEventListener('pointerdown', event => {
    window.__rawUpdateCaptureLog.push('pointerdown@target0');
    target0.setPointerCapture(event.pointerId);
  });
  target0.addEventListener('gotpointercapture', () => {
    window.__rawUpdateCaptureLog.push('gotpointercapture@target0');
  });
  target0.addEventListener('pointerrawupdate', event => {
    window.__rawUpdateCaptureLog.push('pointerrawupdate@target0');
    target0.releasePointerCapture(event.pointerId);
  });
  target0.addEventListener('lostpointercapture', () => {
    window.__rawUpdateCaptureLog.push('lostpointercapture@target0');
  });
  target0.addEventListener('pointermove', () => {
    window.__rawUpdateCaptureLog.push('pointermove@target0');
  });
  target1.addEventListener('pointermove', () => {
    window.__rawUpdateCaptureLog.push('pointermove@target1');
  });
})()
"#,
    )
    .expect("pointer raw update capture listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should set pending capture");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mousemove", -1, None, 0.0, 0.0)
        .expect("mousemove should dispatch raw update before pointermove");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("mouseup should complete pointer stream");

    let result = vm
        .eval("window.__rawUpdateCaptureLog.join('|')")
        .expect("raw update capture log should evaluate");
    assert_eq!(
        result,
        "pointerdown@target0|gotpointercapture@target0|pointerrawupdate@target0|lostpointercapture@target0|pointermove@target1"
    );
}

#[test]
fn release_pointer_capture_clears_pending_capture_before_got_event() {
    let mut vm = new_rendered_test_vm(
        "https://pointer-capture-release-pending.test/",
        r#"<html><body><div id="target0">first</div><div id="target1">second</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target0 = document.getElementById('target0');
  const target1 = document.getElementById('target1');
  window.__releaseCaptureStarted = false;
  window.__releaseCaptureLog = [];
  try {
    target0.setPointerCapture(1);
  } catch (error) {
    window.__releaseCaptureLog.push(`inactive:${error.name}`);
  }
  for (const target of [target0, target1]) {
    for (const type of ['pointerdown', 'gotpointercapture', 'pointermove', 'pointerup', 'lostpointercapture']) {
      target.addEventListener(type, event => {
        if (event.type === 'pointermove' && !window.__releaseCaptureStarted) {
          return;
        }
        window.__releaseCaptureLog.push(`${event.type}@${target.id}`);
        if (event.type === 'pointerdown' && target === target0) {
          window.__releaseCaptureStarted = true;
          target0.setPointerCapture(event.pointerId);
          target0.releasePointerCapture(event.pointerId);
          window.__releaseCaptureLog.push(`has:${target0.hasPointerCapture(event.pointerId)}`);
        }
      });
    }
  }
})()
"#,
    )
    .expect("release capture listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch pointerdown");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mousemove", -1, None, 0.0, 0.0)
        .expect("uncaptured pointermove should dispatch to hit target");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("uncaptured pointerup should dispatch to hit target");

    let result = vm
        .eval("window.__releaseCaptureLog.join('|')")
        .expect("release capture log should evaluate");
    assert_eq!(
        result,
        "inactive:NotFoundError|pointerdown@target0|has:false|pointermove@target1|pointerup@target1"
    );
}

#[test]
fn removing_got_pointer_capture_target_dispatches_lost_on_document() {
    let mut vm = new_rendered_test_vm(
        "https://pointer-capture-got-removal.test/",
        r#"<html><body><div id="button">button</div><div id="target0">capture</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const button = document.getElementById('button');
  const target0 = document.getElementById('target0');
  window.__captureRemovalLog = [];
  button.addEventListener('pointerdown', event => {
    window.__captureRemovalLog.push('pointerdown@button');
    target0.setPointerCapture(event.pointerId);
  });
  button.addEventListener('pointerup', () => {
    window.__captureRemovalLog.push('pointerup@button');
  });
  target0.addEventListener('gotpointercapture', () => {
    window.__captureRemovalLog.push('gotpointercapture@target0');
    target0.remove();
  });
  target0.addEventListener('lostpointercapture', () => {
    window.__captureRemovalLog.push('lostpointercapture@target0');
  });
  target0.addEventListener('pointerup', () => {
    window.__captureRemovalLog.push('pointerup@target0');
  });
  document.addEventListener('lostpointercapture', event => {
    if (event.target === document) {
      window.__captureRemovalLog.push('lostpointercapture@document');
    }
  });
})()
"#,
    )
    .expect("capture removal listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should set pending pointer capture");
    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("mouseup should process removed capture target");

    let result = vm
        .eval("window.__captureRemovalLog.join('|')")
        .expect("capture removal log should evaluate");
    assert_eq!(
        result,
        "pointerdown@button|gotpointercapture@target0|lostpointercapture@document|pointerup@button"
    );
}

#[test]
fn lost_pointer_capture_can_remove_pending_target_before_got() {
    let mut vm = new_rendered_test_vm(
        "https://pointer-capture-lost-removes-pending.test/",
        r#"<html><body><div id="button">button</div><div id="target0">capture0</div><div id="target1">capture1</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const button = document.getElementById('button');
  const target0 = document.getElementById('target0');
  const target1 = document.getElementById('target1');
  window.__pendingRemovalLog = [];
  button.addEventListener('pointerdown', event => {
    window.__pendingRemovalLog.push('pointerdown@button');
    target0.setPointerCapture(event.pointerId);
  });
  button.addEventListener('pointerup', () => {
    window.__pendingRemovalLog.push('pointerup@button');
  });
  target0.addEventListener('gotpointercapture', () => {
    window.__pendingRemovalLog.push('gotpointercapture@target0');
  });
  target0.addEventListener('pointermove', event => {
    window.__pendingRemovalLog.push('pointermove@target0');
    target1.setPointerCapture(event.pointerId);
  });
  target0.addEventListener('lostpointercapture', () => {
    window.__pendingRemovalLog.push('lostpointercapture@target0');
    target1.remove();
  });
  target1.addEventListener('gotpointercapture', () => {
    window.__pendingRemovalLog.push('gotpointercapture@target1');
  });
})()
"#,
    )
    .expect("pending capture removal listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should set first pending capture");
    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousemove", -1, None, 0.0, 0.0)
        .expect("mousemove should dispatch to first capture target");
    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("mouseup should skip removed second capture target");

    let result = vm
        .eval("window.__pendingRemovalLog.join('|')")
        .expect("pending capture removal log should evaluate");
    assert_eq!(
        result,
        "pointerdown@button|gotpointercapture@target0|pointermove@target0|lostpointercapture@target0|pointerup@button"
    );
}

#[test]
fn removed_pending_pointer_capture_target_is_cleared_immediately() {
    let mut vm = new_rendered_test_vm(
        "https://pointer-capture-pending-removal-hook.test/",
        r#"<html><body><div id="button">button</div><div id="target0">capture</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const button = document.getElementById('button');
  const target0 = document.getElementById('target0');
  window.__pendingHookLog = [];
  button.addEventListener('pointerdown', event => {
    window.__pendingHookLog.push('pointerdown@button');
    target0.setPointerCapture(event.pointerId);
    target0.remove();
    window.__pendingHookLog.push(`has:${target0.hasPointerCapture(event.pointerId)}`);
  });
  button.addEventListener('pointerup', () => {
    window.__pendingHookLog.push('pointerup@button');
  });
  target0.addEventListener('gotpointercapture', () => {
    window.__pendingHookLog.push('gotpointercapture@target0');
  });
  document.addEventListener('lostpointercapture', event => {
    if (event.target === document) {
      window.__pendingHookLog.push('lostpointercapture@document');
    }
  });
})()
"#,
    )
    .expect("pending capture hook listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should set then clear pending pointer capture");
    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("mouseup should not capture removed pending target");

    let result = vm
        .eval("window.__pendingHookLog.join('|')")
        .expect("pending hook log should evaluate");
    assert_eq!(result, "pointerdown@button|has:false|pointerup@button");
}

#[test]
fn removed_active_pointer_capture_target_loses_capture_on_next_event() {
    let mut vm = new_rendered_test_vm(
        "https://pointer-capture-active-removal-hook.test/",
        r#"<html><body><div id="button">button</div><div id="target0">capture</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const button = document.getElementById('button');
  const target0 = document.getElementById('target0');
  window.__activeHookLog = [];
  button.addEventListener('pointerdown', event => {
    window.__activeHookLog.push('pointerdown@button');
    target0.setPointerCapture(event.pointerId);
  });
  button.addEventListener('pointerup', () => {
    window.__activeHookLog.push('pointerup@button');
  });
  target0.addEventListener('gotpointercapture', () => {
    window.__activeHookLog.push('gotpointercapture@target0');
  });
  target0.addEventListener('pointermove', event => {
    window.__activeHookLog.push('pointermove@target0');
    target0.remove();
    window.__activeHookLog.push(`has:${target0.hasPointerCapture(event.pointerId)}`);
  });
  target0.addEventListener('pointerup', () => {
    window.__activeHookLog.push('pointerup@target0');
  });
  document.addEventListener('lostpointercapture', event => {
    if (event.target === document) {
      window.__activeHookLog.push('lostpointercapture@document');
    }
  });
})()
"#,
    )
    .expect("active capture hook listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should set pending pointer capture");
    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousemove", -1, None, 0.0, 0.0)
        .expect("mousemove should dispatch to active capture target");
    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("mouseup should release disconnected capture target on document");

    let result = vm
        .eval("window.__activeHookLog.join('|')")
        .expect("active hook log should evaluate");
    assert_eq!(
        result,
        "pointerdown@button|gotpointercapture@target0|pointermove@target0|has:false|lostpointercapture@document|pointerup@button"
    );
}

#[test]
fn mouse_hover_dispatches_pointer_boundary_before_mouse_boundary() {
    let mut vm = new_rendered_test_vm(
        "https://pointer-boundary-order.test/",
        r#"<html><body><div id="a">a</div><div id="b">b</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  window.__boundaryLog = [];
  for (const id of ['a', 'b']) {
    const target = document.getElementById(id);
    for (const type of [
      'pointerover', 'pointerenter', 'pointerout', 'pointerleave', 'pointermove',
      'mouseover', 'mouseenter', 'mouseout', 'mouseleave', 'mousemove'
    ]) {
      target.addEventListener(type, event => {
        window.__boundaryLog.push([
          event.type,
          id,
          event.pointerType || '',
          event.relatedTarget ? event.relatedTarget.id : '',
          event.bubbles
        ].join(':'));
      });
    }
  }
})()
"#,
    )
    .expect("boundary listener setup should evaluate");

    let pointer = crate::runtime::RendererPointerEventProperties {
        pointer_id: 1,
        pointer_type: "pen".to_owned(),
        pressure: 0.0,
        tangential_pressure: 0.0,
        tilt_x: 0.0,
        tilt_y: 0.0,
        twist: 0.0,
    };

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");
    vm.dispatch_mouse_event_at_point_with_pointer(
        10.0,
        10.0,
        "mousemove",
        0,
        Some(0),
        0,
        0.0,
        0.0,
        pointer.clone(),
    )
    .expect("first mousemove should dispatch boundary events");
    vm.dispatch_mouse_event_at_point_with_pointer(
        10.0,
        34.0,
        "mousemove",
        0,
        Some(0),
        0,
        0.0,
        0.0,
        pointer,
    )
    .expect("second mousemove should dispatch boundary events");

    let result = vm
        .eval("window.__boundaryLog.join('|')")
        .expect("boundary log should evaluate");
    assert_eq!(
        result,
        "pointerover:a:pen::true|pointerenter:a:pen::false|mouseover:a:::true|mouseenter:a:::false|pointermove:a:pen::true|mousemove:a:::true|pointerout:a:pen:b:true|pointerleave:a:pen:b:false|pointerover:b:pen:a:true|pointerenter:b:pen:a:false|mouseout:a::b:true|mouseleave:a::b:false|mouseover:b::a:true|mouseenter:b::a:false|pointermove:b:pen::true|mousemove:b:::true"
    );
}

#[test]
fn mouse_hover_persists_stylo_state_and_reflows_dropdown_on_fresh_paint() {
    let mut vm = new_rendered_test_vm(
        "https://hover-dropdown.test/",
        r#"
<!doctype html>
<style>
  html, body { margin: 0; padding: 0; }
  #menu, #trigger, #submenu, #outside { width: 120px; }
  #trigger, #submenu, #outside { display: block; box-sizing: border-box; height: 24px; }
  #submenu { display: none; }
  #menu:hover #submenu { display: block; }
</style>
<nav id="menu">
  <button id="trigger">menu</button>
  <button id="submenu">child</button>
</nav>
<button id="outside">outside</button>
"#,
    );
    vm.eval(
        r#"
(() => {
  const menu = document.getElementById('menu');
  const trigger = document.getElementById('trigger');
  const submenu = document.getElementById('submenu');
  const outside = document.getElementById('outside');
  window.__submenuClicks = 0;
  window.__duringHover = '';
  window.__afterLeave = '';
  trigger.addEventListener('mousemove', () => {
    window.__duringHover = [
      trigger.matches(':hover'),
      menu.matches(':hover'),
      document.body.matches(':hover'),
      document.documentElement.matches(':hover'),
      getComputedStyle(submenu).display
    ].join('|');
  });
  submenu.addEventListener('click', () => window.__submenuClicks++);
  outside.addEventListener('mousemove', () => {
    window.__afterLeave = [
      trigger.matches(':hover'),
      menu.matches(':hover'),
      submenu.matches(':hover'),
      outside.matches(':hover'),
      getComputedStyle(submenu).display
    ].join('|');
  });
})()
"#,
    )
    .expect("hover dropdown listeners should install");

    assert_eq!(
        vm.eval(
            "[document.getElementById('menu').matches(':hover'), getComputedStyle(document.getElementById('submenu')).display].join('|')"
        )
        .expect("initial hover state should evaluate"),
        "false|none"
    );

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(10.0, 10.0, "mousemove", -1, Some(0), 0.0, 0.0)
        .expect("mousemove should establish hover state");
    assert_eq!(
        vm.eval("window.__duringHover")
            .expect("in-event hover state should evaluate"),
        "true|true|true|true|block"
    );

    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(1920, 1080, 1.0))
        .expect("fresh paint publishes the displayed submenu")
        .expect("document layout");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("mousedown should hit the newly displayed submenu");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("mouseup should activate the newly displayed submenu");
    assert_eq!(
        vm.eval("String(window.__submenuClicks)")
            .expect("submenu click count should evaluate"),
        "1"
    );

    vm.dispatch_mouse_event_at_point(10.0, 60.0, "mousemove", -1, Some(0), 0.0, 0.0)
        .expect("mousemove outside the menu should clear its hover chain");
    assert_eq!(
        vm.eval("window.__afterLeave")
            .expect("post-hover state should evaluate"),
        "false|false|false|true|none"
    );
}
