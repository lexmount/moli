use super::*;
use crate::runtime::{
    RendererDragData, RendererDragDataItem, RendererDraggedFile, RendererInterceptedDrag,
};

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
    vm.dispatch_mouse_event_at_point(25.0, 20.0, "mousemove", 0, Some(1), 0.0, 0.0)
        .expect("first mousemove should start drag");
    vm.dispatch_mouse_event_at_point(25.0, 20.0, "mouseup", 0, Some(0), 0.0, 0.0)
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
    vm.dispatch_mouse_event_at_point(25.0, 20.0, "mousemove", 0, Some(1), 0.0, 0.0)
        .expect("second mousemove should start drag");
    vm.dispatch_mouse_event_at_point(25.0, 20.0, "mouseup", 0, Some(0), 0.0, 0.0)
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
    vm.dispatch_mouse_event_at_point(25.0, 20.0, "mousemove", 0, Some(1), 0.0, 0.0)
        .expect("mousemove should attempt drag");
    vm.dispatch_mouse_event_at_point(25.0, 20.0, "mouseup", 0, Some(0), 0.0, 0.0)
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
fn pointer_capture_lost_dispatches_after_compat_mouseup() {
    let mut vm = new_rendered_test_vm(
        "https://pointer-capture-lost-after-mouseup.test/",
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
        .expect("mouseup should dispatch pointerup, mouseup, lostpointercapture");

    let result = vm
        .eval("window.__captureMouseupOrder.join('|')")
        .expect("capture mouseup order log should evaluate");
    assert_eq!(
        result,
        "pointerdown|gotpointercapture|pointerup|mouseup|lostpointercapture"
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
        false,
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
        false,
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

const DRAG_INTERCEPT_PAGE: &str = r#"<html><body>
<div id="source" draggable="true" style="position:absolute;left:0;top:0;width:100px;height:40px"></div>
<div id="target" style="position:absolute;left:0;top:100px;width:200px;height:100px"></div>
</body></html>"#;

const DRAG_INTERCEPT_LISTENERS: &str = r#"
(() => {
  window.__dragLog = [];
  const source = document.getElementById('source');
  const target = document.getElementById('target');
  source.addEventListener('dragstart', event => {
    event.dataTransfer.setData('text/plain', 'payload');
    event.dataTransfer.setData('application/x-moli', 'custom');
    event.dataTransfer.effectAllowed = 'move';
    window.__dragLog.push('dragstart');
  });
  source.addEventListener('dragend', () => window.__dragLog.push('dragend'));
  target.addEventListener('dragenter', event => {
    window.__dragLog.push('dragenter:' + event.dataTransfer.getData('text/plain'));
  });
  target.addEventListener('dragover', event => {
    event.preventDefault();
    window.__dragLog.push('dragover');
  });
  target.addEventListener('dragleave', () => window.__dragLog.push('dragleave'));
  target.addEventListener('drop', event => {
    event.preventDefault();
    window.__dragLog.push([
      'drop',
      event.dataTransfer.getData('application/x-moli'),
      event.dataTransfer.types.join(','),
      event.dataTransfer.effectAllowed,
    ].join(':'));
  });
})()
"#;

fn dispatch_drag_mouse(
    vm: &mut ScriptVm,
    x: f64,
    y: f64,
    event_name: &str,
    intercept_drags: bool,
) -> crate::runtime::RendererInputDispatchOutcome {
    vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
        x,
        y,
        event_name,
        0,
        Some(if event_name == "mouseup" { 0 } else { 1 }),
        0,
        0.0,
        0.0,
        crate::runtime::RendererPointerEventProperties::default(),
        0,
        intercept_drags,
    )
    .expect("drag mouse event should dispatch")
}

fn start_intercepted_drag(vm: &mut ScriptVm) -> RendererInterceptedDrag {
    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch");
    dispatch_drag_mouse(vm, 20.0, 120.0, "mousemove", true)
        .intercepted_drag
        .expect("an uncanceled dragstart should be reported while intercepting")
}

fn drag_data_from(drag: &RendererInterceptedDrag) -> RendererDragData {
    RendererDragData {
        items: drag.items.clone(),
        files: Vec::new(),
        directories: Vec::new(),
        drag_operations_mask: drag.drag_operations_mask,
    }
}

#[test]
fn native_drag_ignores_replaced_deleted_and_throwing_public_drag_event_constructors() {
    for replacement in [
        "window.DragEvent=function(){window.__constructorReads++;throw Error('author constructor');}",
        "delete window.DragEvent",
        "Object.defineProperty(window,'DragEvent',{configurable:true,get(){window.__constructorReads++;throw Error('author getter');}})",
    ] {
        let mut vm = new_rendered_test_vm("https://drag-intrinsic.test/", DRAG_INTERCEPT_PAGE);
        vm.eval(
            r#"
window.__constructorReads=0;window.__events=[];
const IntrinsicDragEvent=DragEvent;
for(const type of ['dragstart','dragend'])document.addEventListener(type,event=>{
  __events.push([type,event instanceof IntrinsicDragEvent,event.isTrusted]);
  if(type==='dragstart')event.dataTransfer.setData('text/plain','payload');
});
"#,
        )
        .expect("install intrinsic drag listeners");
        vm.eval(replacement)
            .expect("replace the public constructor");
        let drag = start_intercepted_drag(&mut vm);
        assert_eq!(drag.items[0].data, "payload", "{replacement}");
        vm.cancel_dragging()
            .expect("end the intercepted native drag");
        assert_eq!(
            vm.eval("JSON.stringify([__constructorReads,__events])")
                .expect("native constructor observations"),
            r#"[0,[["dragstart",true,true],["dragend",true,true]]]"#,
            "{replacement}"
        );
    }
}

#[test]
fn intercepted_drag_reports_string_data_and_waits_for_drag_events() {
    let mut vm = new_rendered_test_vm("https://drag-intercept-report.test/", DRAG_INTERCEPT_PAGE);
    vm.eval(DRAG_INTERCEPT_LISTENERS)
        .expect("drag listener setup should evaluate");

    let drag = start_intercepted_drag(&mut vm);

    let items: Vec<(&str, &str)> = drag
        .items
        .iter()
        .map(|item| (item.mime_type.as_str(), item.data.as_str()))
        .collect();
    assert_eq!(
        items,
        vec![("text/plain", "payload"), ("application/x-moli", "custom")]
    );
    assert_eq!(drag.drag_operations_mask, 16);
    // The mouse move that started the drag must not also drive dragover.
    assert_eq!(
        vm.eval("window.__dragLog.join('|')").expect("log"),
        "dragstart"
    );

    // Mouse events no longer drive the intercepted drag.
    dispatch_drag_mouse(&mut vm, 20.0, 120.0, "mousemove", true);
    assert_eq!(
        vm.eval("window.__dragLog.join('|')").expect("log"),
        "dragstart"
    );
}

#[test]
fn intercepted_drag_replays_enter_over_drop_and_ends_on_source() {
    let mut vm = new_rendered_test_vm("https://drag-intercept-replay.test/", DRAG_INTERCEPT_PAGE);
    vm.eval(DRAG_INTERCEPT_LISTENERS)
        .expect("drag listener setup should evaluate");
    let data = drag_data_from(&start_intercepted_drag(&mut vm));

    for event in ["dragenter", "dragover", "drop"] {
        let outcome = vm
            .dispatch_drag_event_at_point(20.0, 120.0, event, data.clone(), 0)
            .expect("drag event should dispatch");
        assert!(outcome.handled, "{event} should hit the target");
    }

    assert_eq!(
        vm.eval("window.__dragLog.join('|')").expect("log"),
        "dragstart|dragenter:|dragover|dragover|drop:custom:text/plain,application/x-moli:move|dragend"
    );
}

#[test]
fn intercepted_drag_cancel_dispatches_dragleave_and_dragend() {
    let mut vm = new_rendered_test_vm("https://drag-intercept-cancel.test/", DRAG_INTERCEPT_PAGE);
    vm.eval(DRAG_INTERCEPT_LISTENERS)
        .expect("drag listener setup should evaluate");
    let data = drag_data_from(&start_intercepted_drag(&mut vm));

    for event in ["dragenter", "dragleave"] {
        vm.dispatch_drag_event_at_point(20.0, 120.0, event, data.clone(), 0)
            .expect("drag event should dispatch");
    }
    // The session is over: a late drop must not reach the page as part of it.
    vm.dispatch_mouse_event_at_point(20.0, 120.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("mouseup after cancel");

    assert_eq!(
        vm.eval("window.__dragLog.join('|')").expect("log"),
        "dragstart|dragenter:|dragleave|dragend"
    );
}

#[test]
fn canceled_dragstart_is_not_intercepted() {
    let mut vm = new_rendered_test_vm("https://drag-intercept-canceled.test/", DRAG_INTERCEPT_PAGE);
    vm.eval(
        "document.getElementById('source').addEventListener('dragstart', e => e.preventDefault())",
    )
    .expect("listener setup should evaluate");
    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch");
    let outcome = dispatch_drag_mouse(&mut vm, 20.0, 120.0, "mousemove", true);
    assert_eq!(outcome.intercepted_drag, None);
}

#[test]
fn drag_without_interception_is_not_reported_and_ends_on_source() {
    let mut vm = new_rendered_test_vm("https://drag-no-intercept.test/", DRAG_INTERCEPT_PAGE);
    vm.eval(DRAG_INTERCEPT_LISTENERS)
        .expect("drag listener setup should evaluate");
    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch");
    let outcome = vm
        .dispatch_mouse_event_at_point(20.0, 120.0, "mousemove", 0, Some(1), 0.0, 0.0)
        .expect("mousemove should start the drag");
    assert_eq!(outcome.intercepted_drag, None);
    vm.dispatch_mouse_event_at_point(20.0, 120.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("mouseup should finish the drag");

    assert_eq!(
        vm.eval("window.__dragLog.join('|')").expect("log"),
        "dragstart|dragenter:|dragover|drop:custom:text/plain,application/x-moli:move|dragend"
    );
}

#[test]
fn native_drag_eligibility_ignores_author_getters_and_requires_threshold() {
    for (tag, attribute, expected) in [
        ("div", "", false),
        ("div", "draggable=false", false),
        ("div", "draggable=TRUE", true),
        ("a", "href=/destination", true),
        ("a", "href=/destination draggable=false", false),
        ("img", "src=/image.png", true),
        ("img", "src=/image.png draggable=false", false),
    ] {
        let markup = format!(
            "<html><body style='margin:0'><{tag} id=source {attribute} style='display:block;position:absolute;left:0;top:0;width:100px;height:40px'></{tag}></body></html>"
        );
        let mut vm = new_rendered_test_vm("https://drag-eligibility.test/", &markup);
        vm.eval(
            r#"
window.__starts = 0;
const source = document.getElementById('source');
Object.defineProperty(source, 'draggable', { get() { throw new Error('author getter'); } });
source.addEventListener('dragstart', event => {
  window.__starts++;
  window.__initialEffect = event.dataTransfer.effectAllowed;
});
"#,
        )
        .expect("native source setup");
        dispatch_drag_mouse(&mut vm, 20.0, 20.0, "mousedown", true);
        let before_threshold = dispatch_drag_mouse(&mut vm, 21.0, 21.0, "mousemove", true);
        assert_eq!(before_threshold.intercepted_drag, None);
        assert_eq!(vm.eval("window.__starts").expect("start count"), "0");
        let drag = dispatch_drag_mouse(&mut vm, 24.0, 20.0, "mousemove", true);
        assert_eq!(
            drag.intercepted_drag.is_some(),
            expected,
            "{tag} {attribute}"
        );
        if expected {
            assert_eq!(
                vm.eval("window.__initialEffect").expect("initial effect"),
                "uninitialized"
            );
            let drag = drag.intercepted_drag.expect("native drag");
            if tag == "a" || tag == "img" {
                let expected = if tag == "a" {
                    "https://drag-eligibility.test/destination"
                } else {
                    "https://drag-eligibility.test/image.png"
                };
                assert!(
                    drag.items
                        .iter()
                        .any(|item| { item.mime_type == "text/uri-list" && item.data == expected })
                );
            }
        }
        vm.cancel_dragging().expect("cancel native drag");
    }
}

#[test]
fn native_drag_uses_draggable_ancestor_and_only_a_primary_press() {
    let page = r#"<html><body style="margin:0"><div id="source" draggable="true"
style="position:absolute;left:0;top:0;width:100px;height:40px"><span id="child"
style="display:block;width:100px;height:40px"></span></div></body></html>"#;
    let mut vm = new_rendered_test_vm("https://drag-ancestor.test/", page);
    vm.eval("window.__source=''; document.addEventListener('dragstart', e => window.__source=e.target.id)")
        .expect("source listener");
    assert_eq!(
        dispatch_drag_mouse(&mut vm, 30.0, 20.0, "mousemove", true).intercepted_drag,
        None,
        "a synthetic buttons mask alone must not arm a gesture"
    );
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 2, Some(2), 0.0, 0.0)
        .expect("secondary press");
    assert_eq!(
        vm.dispatch_mouse_event_at_point(30.0, 20.0, "mousemove", 2, Some(2), 0.0, 0.0)
            .expect("secondary move")
            .intercepted_drag,
        None
    );
    vm.dispatch_mouse_event_at_point(30.0, 20.0, "mouseup", 2, Some(0), 0.0, 0.0)
        .expect("secondary release");
    dispatch_drag_mouse(&mut vm, 20.0, 20.0, "mousedown", true);
    assert!(
        dispatch_drag_mouse(&mut vm, 24.0, 20.0, "mousemove", true)
            .intercepted_drag
            .is_some()
    );
    assert_eq!(vm.eval("window.__source").expect("drag source"), "source");
}

#[test]
fn canceled_dragstart_and_cancel_command_consume_the_held_gesture() {
    let mut vm = new_rendered_test_vm("https://drag-cancel-held.test/", DRAG_INTERCEPT_PAGE);
    vm.eval(
        "window.__starts=0; document.getElementById('source').addEventListener('dragstart', e => { window.__starts++; if(window.__starts === 1)e.preventDefault(); })",
    ).expect("cancel start listener");
    dispatch_drag_mouse(&mut vm, 20.0, 20.0, "mousedown", true);
    for y in [25.0, 30.0, 120.0] {
        assert_eq!(
            dispatch_drag_mouse(&mut vm, 20.0, y, "mousemove", true).intercepted_drag,
            None
        );
    }
    assert_eq!(vm.eval("window.__starts").expect("canceled starts"), "1");
    dispatch_drag_mouse(&mut vm, 20.0, 120.0, "mouseup", true);
    dispatch_drag_mouse(&mut vm, 20.0, 20.0, "mousedown", true);
    assert!(
        dispatch_drag_mouse(&mut vm, 20.0, 120.0, "mousemove", true)
            .intercepted_drag
            .is_some()
    );
    vm.eval("window.__ends=0; document.getElementById('source').addEventListener('dragend', () => window.__ends++)")
        .expect("end listener");
    vm.cancel_dragging().expect("cancel active drag");
    vm.cancel_dragging().expect("idle cancel is idempotent");
    assert_eq!(
        dispatch_drag_mouse(&mut vm, 30.0, 120.0, "mousemove", true).intercepted_drag,
        None
    );
    dispatch_drag_mouse(&mut vm, 30.0, 120.0, "mouseup", true);
    assert_eq!(
        vm.eval("window.__starts+':'+window.__ends")
            .expect("counts"),
        "2:1"
    );
    // Even cancellation before crossing the threshold disarms the press.
    dispatch_drag_mouse(&mut vm, 20.0, 20.0, "mousedown", true);
    vm.cancel_dragging().expect("cancel pending gesture");
    assert_eq!(
        dispatch_drag_mouse(&mut vm, 20.0, 120.0, "mousemove", true).intercepted_drag,
        None
    );
}

#[test]
fn intercepted_drop_revalidates_target_and_operations_and_finishes_without_a_hit() {
    for rejection in ["not-canceled", "none", "disallowed", "new-target", "no-hit"] {
        let mut vm = new_rendered_test_vm("https://drag-rejected-drop.test/", DRAG_INTERCEPT_PAGE);
        vm.eval(
            r#"
window.__drops=0; window.__ends=0;
document.getElementById('source').addEventListener('dragstart', e => {
  e.dataTransfer.setData('text/plain','payload'); e.dataTransfer.effectAllowed='copy';
});
document.getElementById('source').addEventListener('dragend', e => {
  window.__ends++; window.__endEffect=e.dataTransfer.dropEffect;
});
const target=document.getElementById('target');
target.addEventListener('dragover', e => {
  if(window.__reject !== 'not-canceled') e.preventDefault();
  if(window.__reject === 'none') e.dataTransfer.dropEffect='none';
  if(window.__reject === 'disallowed') e.dataTransfer.dropEffect='move';
});
document.addEventListener('drop', () => window.__drops++);
const other=document.createElement('div'); other.style='position:absolute;left:300px;top:100px;width:100px;height:100px';
document.body.append(other);
"#,
        ).expect("drop rejection setup");
        vm.publish_layout_for_test().expect("new target geometry");
        let data = drag_data_from(&start_intercepted_drag(&mut vm));
        vm.dispatch_drag_event_at_point(20.0, 120.0, "dragenter", data.clone(), 0)
            .expect("initial accepted target");
        vm.eval(&format!("window.__reject='{rejection}'"))
            .expect("rejection mode");
        let x = match rejection {
            "new-target" => 320.0,
            "no-hit" => -1000.0,
            _ => 20.0,
        };
        vm.dispatch_drag_event_at_point(x, 120.0, "drop", data.clone(), 0)
            .expect("drop attempt");
        assert!(
            vm.active_drag_session.is_none(),
            "{rejection}: finished session"
        );
        assert_eq!(
            vm.eval("window.__drops+':'+window.__ends+':'+window.__endEffect")
                .expect("drop counts"),
            "0:1:none",
            "{rejection}"
        );
        let late = vm
            .dispatch_drag_event_at_point(20.0, 120.0, "drop", data, 0)
            .expect("late drop");
        assert!(
            !late.handled,
            "{rejection}: late drop cannot recreate a session"
        );
        assert_eq!(
            vm.eval("window.__drops+':'+window.__ends")
                .expect("late counts"),
            "0:1"
        );
    }
}

#[test]
fn native_drag_data_store_is_writable_only_at_start_and_readable_only_at_drop() {
    let mut vm = new_rendered_test_vm("https://drag-store-lifecycle.test/", DRAG_INTERCEPT_PAGE);
    vm.eval(
        r#"
window.__phases=[];
window.DataTransfer=function(){ throw new Error('page constructor must not run'); };
const source=document.getElementById('source'), target=document.getElementById('target');
source.addEventListener('dragstart', e => {
  window.__held=e.dataTransfer;
  e.dataTransfer.setData('text/plain','original');
  window.__phases.push('start:'+e.dataTransfer.getData('text/plain'));
});
for(const type of ['dragenter','dragover']) target.addEventListener(type, e => {
  e.dataTransfer.setData('text/plain','overwritten');
  window.__phases.push(type+':'+e.dataTransfer.getData('text/plain')+':'+e.dataTransfer.types.join(','));
  if(type==='dragover')e.preventDefault();
});
target.addEventListener('drop', e => {
  e.preventDefault();
  e.dataTransfer.setData('text/plain','overwritten');
  window.__phases.push('drop:'+e.dataTransfer.getData('text/plain'));
});
source.addEventListener('dragend', e => window.__phases.push('end:'+e.dataTransfer.getData('text/plain')+':'+e.dataTransfer.types.join(',')));
"#,
    ).expect("store lifecycle listeners");
    let data = drag_data_from(&start_intercepted_drag(&mut vm));
    assert_eq!(
        vm.eval("JSON.stringify([window.__held.getData('text/plain'),Array.from(window.__held.types),Array.from(window.__held.items,item => [item.kind,item.type])])")
            .expect("protected source between events"),
        r#"["",["text/plain"],[["string","text/plain"]]]"#
    );
    vm.dispatch_drag_event_at_point(20.0, 120.0, "dragenter", data.clone(), 0)
        .expect("enter");
    vm.dispatch_drag_event_at_point(20.0, 120.0, "drop", data, 0)
        .expect("accepted drop");
    assert_eq!(
        vm.eval("window.__phases.join('|')")
            .expect("store phase log"),
        "start:original|dragenter::text/plain|dragover::text/plain|drop:original|end::text/plain"
    );
    assert_eq!(vm.eval("window.__held.setData('text/plain','late'); window.__held.getData('text/plain')+':'+window.__held.types.length+':'+window.__held.items.length+':'+window.__held.files.length").expect("disabled store"), ":0:0:0");
}

#[test]
fn native_selection_drag_requires_press_inside_selected_text() {
    let page = r#"<html><body style="margin:0"><div id="source"
style="position:absolute;left:0;top:0;width:400px;height:50px;font:20px monospace">selected text</div></body></html>"#;
    let mut vm = new_rendered_test_vm("https://drag-selection.test/", page);
    let point: Vec<f64> = serde_json::from_str(
        &vm.eval(
            r#"
const text=document.getElementById('source').firstChild;
const range=document.createRange();range.setStart(text,0);range.setEnd(text,8);
const selection=window.getSelection();selection.removeAllRanges();selection.addRange(range);
const rect=range.getBoundingClientRect();JSON.stringify([rect.left+2,rect.top+2]);
"#,
        )
        .expect("selection geometry"),
    )
    .expect("point coordinates");
    vm.publish_layout_for_test().expect("selected layout");
    dispatch_drag_mouse(&mut vm, 390.0, point[1], "mousedown", true);
    assert_eq!(
        dispatch_drag_mouse(&mut vm, 395.0, point[1], "mousemove", true).intercepted_drag,
        None
    );
    dispatch_drag_mouse(&mut vm, 395.0, point[1], "mouseup", true);
    dispatch_drag_mouse(&mut vm, point[0], point[1], "mousedown", true);
    let drag = dispatch_drag_mouse(&mut vm, point[0] + 5.0, point[1], "mousemove", true)
        .intercepted_drag
        .expect("selected text is a native drag source");
    let plain = drag
        .items
        .iter()
        .find(|item| item.mime_type == "text/plain")
        .expect("selected text supplies a plain-text representation");
    assert_eq!(plain.data, "selected");
    assert!(drag.items.iter().any(|item| item.mime_type == "text/html"));
}

#[test]
fn intercepted_drag_replay_replaces_enter_and_drop_data_including_empty_enter() {
    for empty_enter in [true, false] {
        let mut vm = new_rendered_test_vm("https://drag-client-payload.test/", DRAG_INTERCEPT_PAGE);
        vm.eval(
            r#"
window.__ends=0;
document.getElementById('source').addEventListener('dragstart', e => {
  e.dataTransfer.effectAllowed='copy';
  e.dataTransfer.setData('text/plain','original');
  window.__sourceTransfer=e.dataTransfer;
  window.__oldItem=e.dataTransfer.items[0];
});
document.getElementById('source').addEventListener('dragend', e => {
  window.__ends++;
  window.__sameSourceAtEnd=e.dataTransfer===window.__sourceTransfer;
});
document.getElementById('target').addEventListener('dragenter', e => {
  window.__enterTypes=Array.from(e.dataTransfer.types);
  window.__enterRead=e.dataTransfer.getData('text/plain');
});
document.getElementById('target').addEventListener('dragover', e => e.preventDefault());
document.getElementById('target').addEventListener('drop', e => {
  e.preventDefault();
  window.__seen=[e.dataTransfer===window.__sourceTransfer,
    e.dataTransfer.getData('text/plain'),window.__oldItem.type,
    Array.from(e.dataTransfer.types),e.dataTransfer.dropEffect];
});
"#,
        )
        .expect("client replay listeners");
        let original = drag_data_from(&start_intercepted_drag(&mut vm));
        assert_eq!(original.items.len(), 1);
        let mut enter = original.clone();
        if empty_enter {
            enter.items.clear();
        } else {
            enter.items[0].data = "client-enter".to_owned();
        }
        vm.dispatch_drag_event_at_point(20.0, 120.0, "dragenter", enter, 0)
            .expect("replay enter");
        let mut drop_data = original;
        drop_data.items[0].data = "client-drop".to_owned();
        vm.dispatch_drag_event_at_point(20.0, 120.0, "drop", drop_data, 0)
            .expect("replay drop");
        let result: serde_json::Value = serde_json::from_str(
            &vm.eval("JSON.stringify([window.__enterTypes,window.__enterRead,window.__seen,window.__ends,window.__sameSourceAtEnd,window.__sourceTransfer.items.length])")
                .expect("replay result"),
        ).expect("replay JSON");
        let enter_types = if empty_enter {
            Vec::<&str>::new()
        } else {
            vec!["text/plain"]
        };
        assert_eq!(
            result,
            serde_json::json!([
                enter_types,
                "",
                [false, "client-drop", "text/plain", ["text/plain"], "copy"],
                1,
                true,
                0
            ])
        );
    }
}

#[test]
fn native_drop_defaults_accept_editable_targets_without_canceling_dragover() {
    for (markup, mime_type, effect_allowed, accepted) in [
        ("<input id=target>", "text/plain", "copy", true),
        (
            "<textarea id=target></textarea>",
            "text/plain",
            "copy",
            true,
        ),
        (
            "<div id=target contenteditable=true></div>",
            "text/plain",
            "copy",
            true,
        ),
        (
            "<div contenteditable=true><span id=target></span></div>",
            "text/plain",
            "copy",
            true,
        ),
        ("<input id=target>", "text/plain", "copyMove", true),
        (
            "<textarea id=target></textarea>",
            "text/plain",
            "copyMove",
            true,
        ),
        (
            "<div id=target contenteditable=true></div>",
            "text/plain",
            "copyMove",
            true,
        ),
        ("<input id=target>", "text/plain", "linkMove", false),
        (
            "<textarea id=target></textarea>",
            "text/plain",
            "linkMove",
            false,
        ),
        (
            "<div id=target contenteditable=true></div>",
            "text/plain",
            "linkMove",
            false,
        ),
        ("<input id=target readonly>", "text/plain", "copy", false),
        ("<input id=target disabled>", "text/plain", "copy", false),
        (
            "<fieldset disabled><input id=target></fieldset>",
            "text/plain",
            "copy",
            false,
        ),
        (
            "<textarea id=target readonly></textarea>",
            "text/plain",
            "copy",
            false,
        ),
        (
            "<textarea id=target disabled></textarea>",
            "text/plain",
            "copy",
            false,
        ),
        (
            "<div contenteditable=true><input id=target readonly></div>",
            "text/plain",
            "copy",
            false,
        ),
        (
            "<div contenteditable=true><span id=target contenteditable=false></span></div>",
            "text/plain",
            "copy",
            false,
        ),
        ("<div id=target></div>", "text/plain", "copy", false),
        ("<input id=target>", "text/html", "copy", false),
        ("<input id=target>", "text/plain", "none", false),
    ] {
        let page = format!(
            r#"<html><head><style>
body {{ margin:0 }}
#source {{ position:absolute;left:0;top:0;width:100px;height:40px }}
#target {{ display:block;position:absolute;left:0;top:100px;width:200px;height:80px }}
</style></head><body><div id=source draggable=true></div>{markup}</body></html>"#
        );
        let mut vm = new_rendered_test_vm("https://drag-editable-default.test/", &page);
        vm.eval(&format!(
            r#"
window.__drops=0;window.__ends=0;window.__protectedReads='';
window.__dropTarget=document.getElementById('target');
window.__dropHost=window.__dropTarget.closest('[contenteditable]') || window.__dropTarget;
document.getElementById('source').addEventListener('dragstart', e => {{
  e.dataTransfer.setData('{mime_type}','payload');
  e.dataTransfer.effectAllowed='{effect_allowed}';
}});
document.getElementById('source').addEventListener('dragend', e => {{
  window.__ends++;window.__endEffect=e.dataTransfer.dropEffect;
}});
document.getElementById('target').addEventListener('dragover', e => {{
  window.__protectedReads+=e.dataTransfer.getData('text/plain');
}});
document.addEventListener('drop', () => window.__drops++);
"#
        ))
        .expect("editable drop listeners");
        let data = drag_data_from(&start_intercepted_drag(&mut vm));
        vm.dispatch_drag_event_at_point(20.0, 120.0, "dragenter", data.clone(), 0)
            .expect("editable target enter");
        vm.dispatch_drag_event_at_point(20.0, 120.0, "drop", data, 0)
            .expect("editable target drop");
        let observed: serde_json::Value = serde_json::from_str(
            &vm.eval(
                r#"(() => {
const target=window.__dropTarget;
const host=window.__dropHost;
const value='value' in target ? target.value : host.textContent;
return JSON.stringify([window.__drops,window.__ends,value,window.__protectedReads,window.__endEffect]);
})()"#,
            )
            .expect("editable drop result"),
        )
        .expect("editable result JSON");
        assert_eq!(
            observed,
            serde_json::json!([
                usize::from(accepted),
                1,
                if accepted { "payload" } else { "" },
                "",
                if accepted { "copy" } else { "none" }
            ]),
            "{markup}, {mime_type}, effectAllowed={effect_allowed}"
        );
    }
}

#[test]
fn successful_native_drag_suppresses_pointer_input_until_the_drag_finishes() {
    let mut vm = new_rendered_test_vm("https://drag-pointer-stream.test/", DRAG_INTERCEPT_PAGE);
    vm.eval(
        r#"
window.__ordinary=[];window.__dragPointerOrder=[];window.__cancelState=[];
const source=document.getElementById('source');
source.addEventListener('pointerdown', e => source.setPointerCapture(e.pointerId));
source.addEventListener('dragstart', e => e.dataTransfer.setData('text/plain','payload'));
for(const type of ['pointerdown','mousedown','pointermove','mousemove','pointerup','mouseup','click']) {
  document.addEventListener(type, () => window.__ordinary.push(type));
}
for(const type of ['dragstart','pointercancel','lostpointercapture','dragend']) {
  document.addEventListener(type, e => {
    window.__dragPointerOrder.push(type);
    if(type==='pointercancel') {
      window.__cancelState.push([type,e.cancelable,e.buttons,e.pressure,source.hasPointerCapture(e.pointerId)]);
    } else if(type==='lostpointercapture') {
      window.__cancelState.push([type,e.cancelable,e.buttons,source.hasPointerCapture(e.pointerId)]);
    }
  });
}
"#,
    ).expect("pointer stream listeners");
    dispatch_drag_mouse(&mut vm, 20.0, 20.0, "mousedown", true);
    let outcome = dispatch_drag_mouse(&mut vm, 20.0, 120.0, "mousemove", true);
    let data = drag_data_from(&outcome.intercepted_drag.expect("successful native drag"));
    assert_eq!(
        vm.eval("window.__dragPointerOrder.join('|')")
            .expect("start order"),
        "dragstart|pointercancel|lostpointercapture"
    );
    assert_eq!(
        vm.eval("JSON.stringify(window.__cancelState)")
            .expect("cancel state"),
        "[[\"pointercancel\",false,0,0,true],[\"lostpointercapture\",false,0,false]]"
    );
    let ordinary_at_start = vm
        .eval("window.__ordinary.join('|')")
        .expect("initial stream");
    assert_eq!(
        ordinary_at_start,
        "pointerdown|mousedown|pointermove|mousemove"
    );
    dispatch_drag_mouse(&mut vm, 20.0, 125.0, "mousemove", true);
    assert_eq!(
        vm.eval("window.__ordinary.join('|')")
            .expect("active drag suppresses input"),
        ordinary_at_start
    );
    vm.dispatch_drag_event_at_point(20.0, 125.0, "dragcancel", data, 0)
        .expect("end the intercepted drag");
    // Chromium resumes explicitly injected ordinary input after dragCancel,
    // even when the client still reports the physical button as held.
    dispatch_drag_mouse(&mut vm, 20.0, 130.0, "mousemove", true);
    dispatch_drag_mouse(&mut vm, 20.0, 130.0, "mouseup", true);
    assert_eq!(
        vm.eval("window.__ordinary.join('|')")
            .expect("ordinary input after cancellation"),
        format!("{ordinary_at_start}|pointermove|mousemove|pointerup|mouseup")
    );
    assert_eq!(
        vm.eval("window.__dragPointerOrder.join('|')")
            .expect("final order"),
        "dragstart|pointercancel|lostpointercapture|dragend"
    );
    dispatch_drag_mouse(&mut vm, 20.0, 20.0, "mousedown", true);
    dispatch_drag_mouse(&mut vm, 20.0, 20.0, "mouseup", true);
    assert_eq!(
        vm.eval("window.__ordinary.slice(-5).join('|')")
            .expect("fresh gesture"),
        "pointerdown|mousedown|pointerup|mouseup|click"
    );
}

#[test]
fn canceled_native_dragstart_preserves_the_pointer_stream_and_capture_until_release() {
    let mut vm = new_rendered_test_vm(
        "https://drag-pointer-start-canceled.test/",
        DRAG_INTERCEPT_PAGE,
    );
    vm.eval(
        r#"
window.__counts={};window.__captureLost=false;
const source=document.getElementById('source');
source.addEventListener('pointerdown', e => source.setPointerCapture(e.pointerId));
source.addEventListener('dragstart', e => e.preventDefault());
source.addEventListener('lostpointercapture', e => {
  window.__captureLost=!source.hasPointerCapture(e.pointerId);
});
for(const type of ['pointermove','mousemove','pointerup','mouseup','dragstart','pointercancel','lostpointercapture','dragend']) {
  document.addEventListener(type, () => window.__counts[type]=(window.__counts[type] || 0)+1);
}
"#,
    ).expect("canceled pointer stream listeners");
    dispatch_drag_mouse(&mut vm, 20.0, 20.0, "mousedown", true);
    let outcome = dispatch_drag_mouse(&mut vm, 20.0, 120.0, "mousemove", true);
    assert!(
        outcome.intercepted_drag.is_none(),
        "canceled start is not intercepted"
    );
    dispatch_drag_mouse(&mut vm, 20.0, 125.0, "mousemove", true);
    dispatch_drag_mouse(&mut vm, 20.0, 125.0, "mouseup", true);
    assert_eq!(
        vm.eval("JSON.stringify(['pointermove','mousemove','pointerup','mouseup','dragstart','pointercancel','lostpointercapture','dragend'].map(type => window.__counts[type] || 0))")
            .expect("ordinary stream counts"),
        "[2,2,1,1,1,0,1,0]"
    );
    assert_eq!(
        vm.eval("window.__captureLost").expect("capture released"),
        "true"
    );
}

#[test]
fn cdp_drag_payload_replacement_and_event_view_lifetimes_are_independent() {
    for empty_enter in [false, true] {
        let mut vm = new_rendered_test_vm("https://drag-payload-views.test/", DRAG_INTERCEPT_PAGE);
        vm.eval(
            r#"
window.__views=[];window.__records=[];window.__priorStates=[];window.__ends=0;
const source=document.getElementById('source'),target=document.getElementById('target');
source.addEventListener('dragstart', e => {
  e.dataTransfer.setData('text/plain','source-text');
  e.dataTransfer.setData('application/x-source','source-metadata');
  e.dataTransfer.effectAllowed='copyMove';
  window.__sourceTransfer=e.dataTransfer;
  window.__sourceItem=e.dataTransfer.items[0];
  window.__sourceTypes=e.dataTransfer.types;
});
source.addEventListener('dragend', e => {
  window.__ends++;
  window.__sourceEnd=[e.dataTransfer===window.__sourceTransfer,e.cancelable,
    e.dataTransfer.getData('text/plain'),e.dataTransfer.types.length,
    e.dataTransfer.items.length,e.dataTransfer.files.length,
    e.dataTransfer.effectAllowed,e.dataTransfer.dropEffect];
});
for(const type of ['dragenter','dragover','drop']) target.addEventListener(type, e => {
  window.__priorStates.push(window.__views.map(view => [view.getData('text/plain'),
    view.types.length,view.items.length,view.files.length]));
  const view=e.dataTransfer;
  window.__views.push(view);
  view.setData('text/plain','must-not-replace');
  view.effectAllowed='none';
  window.__records.push({
    type,allowed:view.effectAllowed,effect:view.dropEffect,
    data:[view.getData('text/plain'),view.getData('application/x-enter'),
      view.getData('application/x-ignored'),view.getData('application/x-drop')],
    types:Array.from(view.types),
    items:Array.from(view.items,item => [item.kind,item.type]),
    files:Array.from(view.files,file => [file.name,file.type,file.size,file.lastModified]),
    sourceSame:view===window.__sourceTransfer,
    sourceClosed:[window.__sourceTransfer.getData('text/plain'),
      window.__sourceTransfer.types.length,window.__sourceTransfer.items.length,
      window.__sourceItem.kind,window.__sourceItem.type]
  });
  if(type==='dragover') {
    e.preventDefault();
    view.dropEffect=view.effectAllowed==='linkMove' ? 'move' : 'copy';
  }
  if(type==='drop') {
    e.preventDefault();
    window.__savedDropFile=view.files[0];
    window.__savedDropFile.text().then(text => window.__fileText=text);
  }
});
"#,
        )
        .expect("payload and view listeners");
        let intercepted = start_intercepted_drag(&mut vm);
        assert_eq!(intercepted.drag_operations_mask, 17);
        assert_eq!(intercepted.items.len(), 2);
        assert_eq!(intercepted.items[0].data, "source-text");
        assert_eq!(intercepted.items[1].mime_type, "application/x-source");
        assert_eq!(intercepted.items[1].data, "source-metadata");
        let payload = |name: &str, mask| RendererDragData {
            items: vec![
                RendererDragDataItem {
                    mime_type: "text/plain".to_owned(),
                    data: format!("{name}-text"),
                    title: None,
                    base_url: None,
                },
                RendererDragDataItem {
                    mime_type: format!("application/x-{name}"),
                    data: format!("{name}-metadata"),
                    title: None,
                    base_url: None,
                },
            ],
            files: vec![RendererDraggedFile {
                name: format!("{name}.bin"),
                mime_type: "application/octet-stream".to_owned(),
                bytes: name.as_bytes().to_vec(),
                last_modified: 123.0,
            }],
            directories: Vec::new(),
            drag_operations_mask: mask,
        };
        let mut enter = payload("enter", 1);
        if empty_enter {
            enter.items.clear();
            enter.files.clear();
        }
        vm.dispatch_drag_event_at_point(20.0, 120.0, "dragenter", enter, 0)
            .expect("enter payload");
        vm.dispatch_drag_event_at_point(20.0, 120.0, "dragover", payload("ignored", 18), 0)
            .expect("over changes operations only");
        vm.dispatch_drag_event_at_point(20.0, 120.0, "drop", payload("drop", 17), 0)
            .expect("drop replaces payload after pre-over");
        let observed: serde_json::Value = serde_json::from_str(&vm.eval(
            "JSON.stringify({records:window.__records,prior:window.__priorStates,distinct:new Set(window.__views).size,ends:window.__ends,end:window.__sourceEnd,sourceTypes:Array.from(window.__sourceTypes),sourceAfter:[window.__sourceTransfer.getData('text/plain'),window.__sourceTransfer.types.length,window.__sourceTransfer.items.length,window.__sourceTransfer.files.length,window.__sourceItem.kind,window.__sourceItem.type],fileBytes:window.__fileText,closed:window.__views.map(view => [view.getData('text/plain'),view.types.length,view.items.length,view.files.length])})",
        ).expect("payload and identity result")).expect("payload JSON");
        let entered_types = if empty_enter {
            serde_json::json!([])
        } else {
            serde_json::json!(["text/plain", "application/x-enter", "Files"])
        };
        let entered_items = if empty_enter {
            serde_json::json!([])
        } else {
            serde_json::json!([
                ["string", "text/plain"],
                ["string", "application/x-enter"],
                ["file", "application/octet-stream"]
            ])
        };
        let mut expected_records = Vec::new();
        for (event, allowed, effect) in [
            ("dragenter", "copy", "copy"),
            ("dragover", "linkMove", "move"),
            ("dragover", "copyMove", "move"),
        ] {
            expected_records.push(serde_json::json!({
                "type":event,"allowed":allowed,"effect":effect,"data":["","","",""],
                "types":entered_types,"items":entered_items,"files":[],"sourceSame":false,
                "sourceClosed":["",2,2,"string","text/plain"]
            }));
        }
        expected_records.push(serde_json::json!({
            "type":"drop","allowed":"copyMove","effect":"copy",
            "data":["drop-text","","","drop-metadata"],
            "types":["text/plain","application/x-drop","Files"],
            "items":[["string","text/plain"],["string","application/x-drop"],["file","application/octet-stream"]],
            "files":[["drop.bin","application/octet-stream",4,123]],"sourceSame":false,
            "sourceClosed":["",2,2,"string","text/plain"]
        }));
        assert_eq!(
            observed,
            serde_json::json!({
                "records":expected_records,
                "prior":[[],[["",0,0,0]],[["",0,0,0],["",0,0,0]],[["",0,0,0],["",0,0,0],["",0,0,0]]],
                "distinct":4,"ends":1,"end":[true,false,"",2,2,0,"copyMove","copy"],
                "sourceTypes":["text/plain","application/x-source"],
                "sourceAfter":["",0,0,0,"",""],
                "fileBytes":"drop",
                "closed":[["",0,0,0],["",0,0,0],["",0,0,0],["",0,0,0]]
            }),
            "empty_enter={empty_enter}"
        );
    }
}

#[test]
fn native_drag_target_transitions_share_enter_leave_view_and_preserve_related_targets() {
    let page = r#"<html><body style="margin:0">
<div id=source draggable=true style="position:absolute;left:0;top:0;width:100px;height:40px"></div>
<div id=a style="position:absolute;left:0;top:100px;width:150px;height:100px"></div>
<div id=b style="position:absolute;left:200px;top:100px;width:150px;height:100px"></div>
</body></html>"#;
    let mut vm = new_rendered_test_vm("https://drag-target-transitions.test/", page);
    vm.eval(
        r#"
window.__views=[];window.__events=[];window.__sourceTransfer=null;
const source=document.getElementById('source');
source.addEventListener('dragstart', e => {
  window.__sourceTransfer=e.dataTransfer;
  e.dataTransfer.setData('text/plain','source');e.dataTransfer.effectAllowed='copy';
});
function record(e) {
  const transfer=e.dataTransfer;
  if(!window.__views.includes(transfer))window.__views.push(transfer);
  if(e.type==='dragenter' && e.target.id==='b')transfer.dropEffect='copy';
  e.preventDefault();
  window.__events.push([e.type,e.target.id,e.relatedTarget ? e.relatedTarget.id : null,
    e.cancelable,e.defaultPrevented,window.__views.indexOf(transfer),
    transfer===window.__sourceTransfer,transfer.getData('text/plain'),transfer.dropEffect]);
}
for(const id of ['a','b']) {
  const target=document.getElementById(id);
  for(const type of ['dragenter','dragleave','dragover','drop'])target.addEventListener(type,record);
}
source.addEventListener('dragend',record);
"#,
    ).expect("target transition listeners");
    let mut data = drag_data_from(&start_intercepted_drag(&mut vm));
    vm.dispatch_drag_event_at_point(20.0, 120.0, "dragenter", data.clone(), 0)
        .expect("first enter A");
    vm.dispatch_drag_event_at_point(220.0, 120.0, "dragover", data.clone(), 0)
        .expect("move A to B");
    vm.dispatch_drag_event_at_point(20.0, 120.0, "dragover", data.clone(), 0)
        .expect("move B to A");
    vm.dispatch_drag_event_at_point(20.0, 120.0, "dragover", data.clone(), 0)
        .expect("over A without transition");
    data.items[0].data = "target-final".to_owned();
    vm.dispatch_drag_event_at_point(20.0, 120.0, "drop", data, 0)
        .expect("drop A");
    let observed: serde_json::Value = serde_json::from_str(&vm.eval(
        "JSON.stringify([window.__events,window.__views.map(view => [view.getData('text/plain'),view.types.length,view.items.length,view.files.length])])",
    ).expect("target transition result")).expect("transition JSON");
    assert_eq!(
        observed,
        serde_json::json!([
            [
                ["dragenter", "a", null, true, true, 0, false, "", "copy"],
                ["dragenter", "b", "a", true, true, 1, false, "", "copy"],
                ["dragleave", "a", "b", false, false, 1, false, "", "none"],
                ["dragenter", "a", "b", true, true, 2, false, "", "copy"],
                ["dragleave", "b", "a", false, false, 2, false, "", "none"],
                ["dragover", "a", null, true, true, 3, false, "", "copy"],
                ["dragover", "a", null, true, true, 4, false, "", "copy"],
                [
                    "drop",
                    "a",
                    null,
                    true,
                    true,
                    5,
                    false,
                    "target-final",
                    "copy"
                ],
                ["dragend", "source", null, false, false, 6, true, "", "copy"]
            ],
            [
                ["", 0, 0, 0],
                ["", 0, 0, 0],
                ["", 0, 0, 0],
                ["", 0, 0, 0],
                ["", 0, 0, 0],
                ["", 0, 0, 0],
                ["", 0, 0, 0]
            ]
        ])
    );
}

#[test]
fn native_selection_drag_reuses_selection_serializer_without_author_callbacks() {
    let page = r#"<html><body style="margin:0"><div id=source style="font:20px/24px monospace;width:400px"><div id=first>alpha<span style="display:none">display-omitted</span></div><div>beta<br>gamma</div></div></body></html>"#;
    let mut vm = new_rendered_test_vm("https://drag-selection-serializer.test/", page);
    let (selected_text, point, raw_text): (String, Vec<f64>, String) = serde_json::from_str(
        &vm.eval(
            r#"
const source=document.getElementById('source');
const range=document.createRange();range.selectNodeContents(source);
const selection=getSelection();selection.removeAllRanges();selection.addRange(range);
const selectedText=selection.toString();
const firstTextRange=document.createRange();
firstTextRange.selectNodeContents(document.getElementById('first').firstChild);
const rect=firstTextRange.getBoundingClientRect();
window.__selectionStringCalls=0;
Selection.prototype.toString=Range.prototype.toString=function() {
  window.__selectionStringCalls++;
  throw new Error('native selection dragging must not call author toString');
};
JSON.stringify([selectedText,[rect.left+2,rect.top+2],source.textContent]);
"#,
        )
        .expect("select visible text and replace public stringifiers"),
    )
    .expect("selection text and hit point");
    let alpha = selected_text.find("alpha").expect("first visible block");
    let beta = selected_text.find("beta").expect("second visible block");
    let gamma = selected_text.find("gamma").expect("visible text after br");
    assert!(alpha < beta && beta < gamma, "{selected_text:?}");
    assert!(
        raw_text.contains("display-omitted"),
        "fixture includes hidden text"
    );
    assert!(
        !selected_text.contains("display-omitted"),
        "Selection excludes display:none text"
    );
    vm.publish_layout_for_test()
        .expect("publish selected text geometry");
    dispatch_drag_mouse(&mut vm, point[0], point[1], "mousedown", true);
    let drag = dispatch_drag_mouse(&mut vm, point[0] + 5.0, point[1], "mousemove", true)
        .intercepted_drag
        .expect("visible selection starts a native drag");
    let plain = drag
        .items
        .iter()
        .find(|item| item.mime_type == "text/plain")
        .expect("selected text supplies a plain-text representation");
    assert_eq!(plain.data, selected_text);
    assert!(drag.items.iter().any(|item| item.mime_type == "text/html"));
    vm.cancel_dragging().expect("finish selected text drag");
    assert_eq!(
        vm.eval("String(window.__selectionStringCalls)")
            .expect("author stringifier invocation count"),
        "0"
    );
}
