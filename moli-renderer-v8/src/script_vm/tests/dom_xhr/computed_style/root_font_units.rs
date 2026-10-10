use super::*;

#[test]
fn root_font_relative_units_follow_each_documents_live_root_style() {
    let mut vm = new_parsed_test_vm(
        "https://root-font-relative-units.test/",
        r#"<!doctype html>
        <html style="font-size: 0.625rem; line-height: 2">
          <body>
            <div id="target" style="width: 8.8rem; height: 1rlh; font-size: 1.4rem"></div>
          </body>
        </html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement;
  const target = document.getElementById('target');
  const values = () => {
    const targetStyle = getComputedStyle(target);
    const targetValues = [targetStyle.width, targetStyle.height, targetStyle.fontSize];
    const rootFontSize = getComputedStyle(root).fontSize;
    return [rootFontSize, ...targetValues].join(',');
  };
  const initial = values();

  root.style.fontSize = '20px';
  const updated = values();

  root.style.fontSize = '0.625rem';
  const selfRelative = values();

  root.style.lineHeight = '3';
  const updatedLineHeight = values();
  root.style.lineHeight = '2';

  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write('<html style="font-size:7px"><body><div id="child" style="width:10rem"></div></body></html>');
  childDocument.close();
  const childWidth = frame.contentWindow
    .getComputedStyle(childDocument.getElementById('child'))
    .width;

  return [initial, updated, selfRelative, updatedLineHeight, childWidth, values()].join('|');
})()
"#,
        )
        .expect("root font-relative units should track isolated live root styles");

    assert_eq!(
        result,
        "10px,88px,20px,14px|20px,176px,40px,28px|10px,88px,20px,14px|10px,88px,30px,14px|70px|10px,88px,20px,14px"
    );
}

#[test]
fn replacing_document_root_replaces_retained_root_font_state() {
    let mut vm = new_parsed_test_vm(
        "https://replaced-root-font-state.test/",
        r#"<!doctype html><html style="font-size:10px"><body>
          <div id="target" style="width:2rem"></div>
        </body></html>"#,
    );

    let initial = vm
        .eval("getComputedStyle(document.getElementById('target')).width")
        .expect("initial descendant should resolve root-relative width");
    assert_eq!(initial, "20px");

    let replaced = vm
        .eval(
            r#"
(() => {
  const nextRoot = document.createElement('html');
  nextRoot.style.fontSize = '7px';
  const nextBody = document.createElement('body');
  const nextTarget = document.createElement('div');
  nextTarget.id = 'next-target';
  nextTarget.style.width = '2rem';
  nextBody.appendChild(nextTarget);
  nextRoot.appendChild(nextBody);
  document.replaceChild(nextRoot, document.documentElement);
  return getComputedStyle(nextTarget).width;
})()
"#,
        )
        .expect("new descendant should replace the retained root state");
    assert_eq!(replaced, "14px");

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
    .expect("viewport device should rebuild after root replacement");
    let rebuilt = vm
        .eval("getComputedStyle(document.getElementById('next-target')).width")
        .expect("rebuilt device should retain the replacement root state");
    assert_eq!(rebuilt, "14px");
}

#[test]
fn root_font_relative_units_survive_viewport_device_rebuilds() {
    let mut vm = new_parsed_test_vm(
        "https://root-rem-device-rebuild.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    let surface = |inner_width| crate::protocol_types::ViewportSurface {
        inner_width,
        inner_height: 600,
        outer_width: inner_width,
        outer_height: 600,
        device_pixel_ratio: 1.0,
        screen_width: 1920,
        screen_height: 1080,
        screen_avail_width: 1920,
        screen_avail_height: 1040,
        ..Default::default()
    };
    vm.set_viewport_surface(Some(surface(1000)))
        .expect("initial viewport should update");

    let wide = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `
    html { font-size: 10px; }
    @media (min-width: 800px) { html { font-size: 20px; } }
    #root-rem-target { width: 2rem; }
  `;
  document.head.appendChild(style);
  const target = document.createElement('div');
  target.id = 'root-rem-target';
  document.body.appendChild(target);
  globalThis.__rootRemTarget = target;
  const width = getComputedStyle(target).width;
  return [width, getComputedStyle(document.documentElement).fontSize].join('|');
})()
"#,
        )
        .expect("wide root rem styles should resolve");
    assert_eq!(wide, "40px|20px");

    vm.set_viewport_surface(Some(surface(500)))
        .expect("narrow viewport should update");
    let narrow = vm
        .eval(
            "[getComputedStyle(__rootRemTarget).width, getComputedStyle(document.documentElement).fontSize].join('|')",
        )
        .expect("narrow root rem styles should resolve");
    assert_eq!(narrow, "20px|10px");

    vm.set_viewport_surface(Some(surface(1000)))
        .expect("restored viewport should update");
    let restored = vm
        .eval(
            "[getComputedStyle(__rootRemTarget).width, getComputedStyle(document.documentElement).fontSize].join('|')",
        )
        .expect("restored root rem styles should resolve");
    assert_eq!(restored, "40px|20px");

    vm.eval("document.documentElement.style.fontSize = '30px'; true")
        .expect("pending root font mutation should apply");
    vm.set_viewport_surface(Some(surface(900)))
        .expect("device should rebuild while the root mutation is pending");
    let pending_mutation = vm
        .eval(
            "[getComputedStyle(__rootRemTarget).width, getComputedStyle(document.documentElement).fontSize].join('|')",
        )
        .expect("descendant should observe the pending root mutation after device rebuild");
    assert_eq!(pending_mutation, "60px|30px");
}
