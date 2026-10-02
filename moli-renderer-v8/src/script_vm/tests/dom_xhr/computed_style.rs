use super::*;

mod child_list;

fn inspector_active_child_window_scope_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let active = crate::native_bridge::active_child_window_handle(scope).is_some();
    rv.set(v8::Boolean::new(scope, active).into());
}

fn child_document_handle_for_frame_id(vm: &ScriptVm, frame_id: &str) -> DomHandle {
    let frame = element_handle_by_id(vm, frame_id);
    vm._context_host
        .borrow()
        .child_browsing_context_document_handle(frame)
        .expect("iframe should have a child document handle")
}

fn element_handle_by_id(vm: &ScriptVm, id: &str) -> DomHandle {
    vm.document_runtime
        .dom_host()
        .dom()
        .nodes()
        .iter()
        .enumerate()
        .find_map(|(index, node)| {
            let element = node.as_element()?;
            (element.attribute("id") == Some(id)).then_some(DomHandle::new(index))
        })
        .unwrap_or_else(|| panic!("detached element #{id} should have a native handle"))
}

fn owner_document_handle_for_element_id(vm: &ScriptVm, id: &str) -> DomHandle {
    let element = element_handle_by_id(vm, id);
    vm.document_runtime
        .dom_host()
        .owner_document_handle(element)
        .unwrap_or_else(|| panic!("detached element #{id} should have an owner document"))
}

fn computed_style_cache_entry_count_for_document(vm: &ScriptVm, document: DomHandle) -> usize {
    vm._context_host
        .borrow()
        .computed_style_cache_entry_count_for_document_for_test(document)
}

fn registered_custom_property_for_document(vm: &ScriptVm, document: DomHandle, name: &str) -> bool {
    vm._context_host
        .borrow()
        .registered_css_custom_property_registration(document, name)
        .is_some()
}

mod advanced_style_values;
mod computed_style_access;
mod content_and_invalidation;
mod cross_document_and_animations;
mod nested_document_invalidation;
mod properties_and_selectors;
mod stylesheet_and_document_lifecycle;

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
