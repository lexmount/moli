use super::*;

#[test]
fn content_alt_counters_compute_from_stylesheets_variables_and_cssom_updates() {
    let mut vm = new_parsed_test_vm(
        "https://content-alt-counter-computed.test/",
        r#"<!doctype html><html><head><style>
          #target {
            --alt: "Chapter " counter(chapter);
            content: "sheet" / "Chapter " counter(chapter);
          }
          #target::before { content: "before" / var(--alt); }
        </style></head><body><div id="target"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const values = [
    getComputedStyle(target).content,
    getComputedStyle(target, '::before').content
  ];
  target.style.setProperty(
    'content', '"inline" / counters(chapter, ".", DECIMAL)', 'important'
  );
  values.push(getComputedStyle(target).content);
  document.styleSheets[0].cssRules[0].style.content =
    '"updated" / counter(chapter, upper-roman)';
  values.push(getComputedStyle(target).content);
  target.style.removeProperty('content');
  values.push(getComputedStyle(target).content);
  target.style.setProperty('--alt', '"Section " counters(chapter, ".")');
  values.push(getComputedStyle(target, '::before').content);
  return values.join('|');
})()
"#,
        )
        .expect("content alternative counters should flow through Stylo computed values");

    assert_eq!(
        result,
        r#""sheet" / "Chapter " counter(chapter)|"before" / "Chapter " counter(chapter)|"inline" / counters(chapter, ".")|"inline" / counters(chapter, ".")|"updated" / counter(chapter, upper-roman)|"before" / "Section " counters(chapter, ".")"#
    );
}

#[test]
fn element_current_css_zoom_observes_fresh_effective_style_for_rendered_boxes() {
    let mut vm = new_parsed_test_vm(
        "https://element-current-css-zoom.test/",
        r#"<!doctype html><html><body>
          <div id="unzoomed"><div id="unzoomed-child"></div></div>
          <div id="outer" style="zoom: 2">
            <div id="inner" style="zoom: 2">
              <div id="rendered-child"></div>
              <div id="non-rendered-child" style="display: none"></div>
            </div>
          </div>
          <div id="hidden-ancestor" style="display: none; zoom: 3">
            <div id="hidden-descendant" style="zoom: 2"></div>
          </div>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const get = id => document.getElementById(id);
  const initial = [
    get('unzoomed').currentCSSZoom,
    get('outer').currentCSSZoom,
    get('inner').currentCSSZoom,
    get('rendered-child').currentCSSZoom,
    get('non-rendered-child').currentCSSZoom,
    get('hidden-descendant').currentCSSZoom
  ];
  get('unzoomed').style.zoom = 2;
  const updated = [
    get('unzoomed').currentCSSZoom,
    get('unzoomed-child').currentCSSZoom
  ];
  get('outer').style.display = 'none';
  const suppressed = [
    get('outer').currentCSSZoom,
    get('inner').currentCSSZoom,
    get('rendered-child').currentCSSZoom
  ];
  const detached = document.createElement('div');
  detached.style.zoom = 4;
  const descriptor = Object.getOwnPropertyDescriptor(
    Element.prototype,
    'currentCSSZoom'
  );
  let incompatible = 'none';
  try {
    descriptor.get.call({});
  } catch (error) {
    incompatible = error.name;
  }
  return JSON.stringify({
    initial,
    updated,
    suppressed,
    detached: detached.currentCSSZoom,
    shape: [
      typeof descriptor.get,
      descriptor.get.length,
      descriptor.enumerable,
      descriptor.configurable,
      Object.hasOwn(get('outer'), 'currentCSSZoom')
    ],
    incompatible
  });
})()
"#,
        )
        .expect("Element.currentCSSZoom should evaluate");

    assert_eq!(
        result,
        r#"{"initial":[1,2,4,4,1,1],"updated":[2,2],"suppressed":[1,1,1],"detached":1,"shape":["function",0,true,true,false],"incompatible":"TypeError"}"#
    );
}

#[test]
fn menu_uses_block_user_agent_display_default() {
    let mut vm = new_parsed_test_vm(
        "https://menu-user-agent-display.test/",
        r#"<!doctype html>
<html><head><style>#overridden { display: inline; }</style></head><body>
  <menu id="default" type="context"></menu>
  <menu id="overridden" type="context"></menu>
</body></html>"#,
    );

    let result = vm
        .eval(
            r#"
[
  getComputedStyle(document.getElementById('default')).display,
  getComputedStyle(document.getElementById('overridden')).display
].join('|')
"#,
        )
        .expect("menu user-agent display should evaluate");

    assert_eq!(result, "block|inline");
}

#[test]
fn rendered_text_elements_use_chromium_user_agent_defaults() {
    let mut vm = new_parsed_test_vm(
        "https://rendered-text-user-agent-defaults.test/",
        "<!doctype html><body><pre id=pre>text</pre><hr id=hr><optgroup id=optgroup></optgroup>",
    );

    let result = vm
        .eval(
            r#"
['pre', 'hr', 'optgroup']
  .map(id => getComputedStyle(document.getElementById(id)).display)
  .join('|')
"#,
        )
        .expect("rendered-text user-agent defaults should evaluate");

    assert_eq!(result, "block|block|block");
}

#[test]
fn flow_content_uses_chromium_user_agent_typography_defaults() {
    let mut vm = new_parsed_test_vm(
        "https://flow-content-user-agent-defaults.test/",
        r#"<!doctype html>
<html><head><style>
body { font: 16px/1.5 sans-serif; }
.title h1 { font-size: 24px; margin: 19.92px 0; }
</style></head><body>
  <header><a class="title" href="/"><h1 id="title">Title</h1></a></header>
  <main>
    <h1 id="h1">Heading 1</h1>
    <h2 id="h2">Heading 2</h2>
    <p id="paragraph"><strong id="strong">Strong</strong> <a id="link" href="/next">link</a><sup id="sup">1</sup></p>
  </main>
</body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const style = id => getComputedStyle(document.getElementById(id));
  const title = style('title');
  const h1 = style('h1');
  const h2 = style('h2');
  const paragraph = style('paragraph');
  const strong = style('strong');
  const link = style('link');
  const sup = style('sup');
  return [
    title.fontSize, title.fontWeight, title.marginBlockStart, title.marginBlockEnd,
    h1.fontSize, h1.fontWeight, h1.marginBlockStart, h1.marginBlockEnd,
    h2.fontSize, h2.fontWeight, h2.marginBlockStart, h2.marginBlockEnd,
    paragraph.marginBlockStart, paragraph.marginBlockEnd,
    strong.fontWeight,
    link.textDecorationLine, link.cursor,
    sup.fontSize
  ].join('|');
})()
"#,
        )
        .expect("flow-content user-agent typography should evaluate");

    assert_eq!(
        result,
        "24px|700|19.92px|19.92px|32px|700|21.44px|21.44px|24px|700|19.92px|19.92px|16px|16px|700|underline|pointer|13.3281px"
    );
}

#[test]
fn center_uses_legacy_centering_user_agent_default() {
    let mut vm = new_parsed_test_vm(
        "https://center-user-agent-default.test/",
        r#"<!doctype html>
<html><head><style>#overridden { text-align: right; }</style></head><body>
  <center id="default">default</center>
  <center id="overridden">overridden</center>
</body></html>"#,
    );

    let result = vm
        .eval(
            r#"
['default', 'overridden']
  .map(id => getComputedStyle(document.getElementById(id)).textAlign)
  .join('|')
"#,
        )
        .expect("center user-agent alignment should evaluate");

    assert_eq!(result, "-moz-center|right");
}

#[test]
fn list_items_and_first_details_summary_use_user_agent_display_defaults() {
    let mut vm = new_parsed_test_vm(
        "https://list-item-user-agent-display.test/",
        r#"<!doctype html>
<html><head><style>#overridden { display: inline; }</style></head><body>
  <li id="item">item</li>
  <li id="overridden">overridden</li>
  <summary id="outside">outside</summary>
  <details id="details">
    <div>leading content</div>
    <summary id="first">first</summary>
    <summary id="second">second</summary>
  </details>
</body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const display = id => getComputedStyle(document.getElementById(id)).display;
  const values = [
    display('item'),
    display('overridden'),
    display('outside'),
    display('details'),
    display('first'),
    display('second')
  ];
  const details = document.getElementById('details');
  details.insertBefore(document.getElementById('second'), document.getElementById('first'));
  values.push(display('first'), display('second'));
  return values.join('|');
})()
"#,
        )
        .expect("list-item user-agent display should evaluate");

    assert_eq!(
        result,
        "list-item|inline|block|block|list-item|block|block|list-item"
    );
}

#[test]
fn table_elements_use_table_user_agent_display_defaults() {
    let mut vm = new_parsed_test_vm(
        "https://table-user-agent-display.test/",
        r#"<!doctype html>
<table id="table"><caption id="caption">caption</caption><colgroup id="colgroup"><col id="col"></colgroup><thead id="head"><tr id="row"><th id="cell">cell</th></tr></thead><tbody id="body"></tbody><tfoot id="foot"></tfoot></table>"#,
    );

    let result = vm
        .eval(
            r#"
['table', 'caption', 'colgroup', 'col', 'head', 'body', 'foot', 'row', 'cell']
  .map(id => getComputedStyle(document.getElementById(id)).display)
  .join('|')
"#,
        )
        .expect("table user-agent displays should evaluate");

    assert_eq!(
        result,
        "table|table-caption|table-column-group|table-column|table-header-group|table-row-group|table-footer-group|table-row|table-cell"
    );
}

#[test]
fn marquee_user_agent_overflow_overrides_author_styles() {
    let mut vm = new_parsed_test_vm(
        "https://marquee-user-agent-overflow.test/",
        r#"<!doctype html>
<marquee style="overflow: visible"></marquee>
<marquee style="overflow: scroll"></marquee>
<marquee style="overflow: clip"></marquee>
<marquee style="overflow: auto"></marquee>"#,
    );

    let result = vm
        .eval(
            r#"
[...document.querySelectorAll('marquee')]
  .map(element => getComputedStyle(element).overflow)
  .join('|')
"#,
        )
        .expect("marquee user-agent overflow should evaluate");

    assert_eq!(result, "hidden|hidden|hidden|hidden");
}

#[test]
fn dialog_user_agent_display_tracks_open_state() {
    let mut vm = new_parsed_test_vm(
        "https://dialog-user-agent-display.test/",
        "<!doctype html><dialog id=target>Dialog</dialog>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const dialog = document.getElementById('target');
  const values = [getComputedStyle(dialog).display];
  dialog.show();
  values.push(getComputedStyle(dialog).display);
  dialog.close();
  values.push(getComputedStyle(dialog).display);
  return values.join('|');
})()
"#,
        )
        .expect("dialog user-agent display should evaluate");

    assert_eq!(result, "none|block|none");
}

#[test]
fn popover_user_agent_display_tracks_open_state() {
    let mut vm = new_parsed_test_vm(
        "https://popover-user-agent-display.test/",
        "<!doctype html><tool-tip id=target popover=manual>Tooltip</tool-tip>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const popover = document.getElementById('target');
  const values = [getComputedStyle(popover).display];
  popover.showPopover();
  values.push(getComputedStyle(popover).display);
  popover.hidePopover();
  values.push(getComputedStyle(popover).display);
  return values.join('|');
})()
"#,
        )
        .expect("popover user-agent display should evaluate");

    assert_eq!(result, "none|block|none");
}

#[test]
fn dialog_user_agent_colors_follow_its_color_scheme() {
    let mut vm = new_parsed_test_vm(
        "https://dialog-user-agent-colors.test/",
        r#"<!doctype html>
<style>:root { color: CanvasText; background-color: Canvas }</style>
<dialog id="default" open></dialog>
<dialog id="light" open style="color-scheme: only light"></dialog>
<dialog id="dark" open style="color-scheme: only dark"></dialog>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const root = getComputedStyle(document.documentElement);
  const fallback = getComputedStyle(document.getElementById('default'));
  const light = getComputedStyle(document.getElementById('light'));
  const dark = getComputedStyle(document.getElementById('dark'));
  return JSON.stringify({
    defaultMatchesRoot:
      fallback.color === root.color && fallback.backgroundColor === root.backgroundColor,
    schemesDiffer:
      light.color !== dark.color && light.backgroundColor !== dark.backgroundColor
  });
})()
"#,
        )
        .expect("dialog user-agent colors should evaluate");

    assert_eq!(
        result,
        r#"{"defaultMatchesRoot":true,"schemesDiffer":true}"#
    );
}

#[test]
fn modal_dialog_user_agent_visibility_overrides_inheritance() {
    let mut vm = new_parsed_test_vm(
        "https://modal-dialog-user-agent-visibility.test/",
        r#"<!doctype html>
<div style="visibility: hidden"><dialog id="target">Dialog</dialog></div>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const dialog = document.getElementById('target');
  dialog.show();
  const values = [getComputedStyle(dialog).visibility];
  dialog.close();

  dialog.showModal();
  values.push(getComputedStyle(dialog).visibility);
  dialog.close();

  dialog.style.visibility = 'hidden';
  dialog.showModal();
  values.push(getComputedStyle(dialog).visibility);
  return values.join('|');
})()
"#,
        )
        .expect("modal dialog user-agent visibility should evaluate");

    assert_eq!(result, "hidden|visible|hidden");
}

#[test]
fn modal_dialog_top_layer_adjusts_non_absolute_positions() {
    let mut vm = new_parsed_test_vm(
        "https://modal-dialog-top-layer-position.test/",
        r#"<!doctype html><dialog id="target"></dialog>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const dialog = document.getElementById('target');
  const values = [];
  for (const position of ['static', 'relative', 'sticky', 'absolute', 'fixed']) {
    dialog.style.position = position;
    values.push(getComputedStyle(dialog).position);
    dialog.showModal();
    values.push(getComputedStyle(dialog).position);
    dialog.close();
    values.push(getComputedStyle(dialog).position);
  }
  return values.join('|');
})()
"#,
        )
        .expect("modal dialog top-layer position should evaluate");

    assert_eq!(
        result,
        "static|absolute|static|relative|absolute|relative|sticky|absolute|sticky|\
         absolute|absolute|absolute|fixed|fixed|fixed"
    );
}

#[test]
fn semantic_text_decoration_uses_user_agent_defaults() {
    let mut vm = new_parsed_test_vm(
        "https://semantic-text-decoration.test/",
        r#"<!doctype html>
<style>#overridden { text-decoration: none; }</style>
<u id="u"></u><ins id="ins"></ins>
<s id="s"></s><strike id="strike"></strike><del id="del"></del>
<ins id="overridden"></ins>"#,
    );

    let result = vm
        .eval(
            r#"
['u', 'ins', 's', 'strike', 'del', 'overridden']
  .map(id => getComputedStyle(document.getElementById(id)).textDecorationLine)
  .join('|')
"#,
        )
        .expect("semantic text decoration should evaluate");

    assert_eq!(
        result,
        "underline|underline|line-through|line-through|line-through|none"
    );
}

#[test]
fn computed_style_exposes_root_pointer_events() {
    let mut vm = new_storage_test_vm("https://pointer-events-computed.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.textContent = ':root { pointer-events: none; }';
  head.appendChild(style);
  const computed = getComputedStyle(document.documentElement);
  return [
    computed.getPropertyValue('pointer-events'),
    computed.pointerEvents,
    CSS.supports('pointer-events', 'none')
  ].join('|');
})()
"#,
        )
        .expect("root pointer-events computed style should evaluate");

    assert_eq!(result, "none|none|true");
}

#[test]
fn computed_style_exposes_non_inherited_touch_action() {
    let mut vm = new_parsed_test_vm(
        "https://touch-action-computed.test/",
        r#"<!doctype html>
<style>#parent { touch-action: none; }</style>
<div id="parent"><div id="child"></div></div>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.getElementById('parent');
  const child = document.getElementById('child');
  const initial = getComputedStyle(child);
  const values = [
    'touch-action' in initial,
    'touchAction' in initial,
    CSS.supports('touch-action', 'pan-y pan-x'),
    getComputedStyle(parent).touchAction,
    initial.touchAction
  ];

  child.style.touchAction = 'pinch-zoom pan-y pan-x';
  values.push(child.style.touchAction, getComputedStyle(child).touchAction);
  child.style.touchAction = 'pan-x pan-left';
  values.push(child.style.touchAction);
  child.style.touchAction = 'inherit';
  values.push(getComputedStyle(child).touchAction);
  return values.join('|');
})()
"#,
        )
        .expect("touch-action computed style should evaluate");

    assert_eq!(
        result,
        "true|true|true|none|auto|manipulation|manipulation|manipulation|none"
    );
}

#[test]
fn style_element_type_attribute_uses_raw_exact_css_match() {
    let mut vm = new_parsed_test_vm(
        "https://style-type-attribute.test/",
        r#"<!doctype html>
<html><head>
  <style id="missing">#missing-target { color: rgb(0, 128, 0); }</style>
  <style id="empty" type="">#empty-target { color: rgb(0, 128, 0); }</style>
  <style id="mixed" type="TeXt/CsS">#mixed-target { color: rgb(0, 128, 0); }</style>
  <style id="spaced" type=" text/css ">#spaced-target { color: rgb(0, 128, 0); }</style>
  <style id="parameter" type="text/css; charset=utf-8">#parameter-target { color: rgb(0, 128, 0); }</style>
</head><body>
  <div id="missing-target"></div><div id="empty-target"></div>
  <div id="mixed-target"></div><div id="spaced-target"></div>
  <div id="parameter-target"></div>
</body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const names = ['missing', 'empty', 'mixed', 'spaced', 'parameter'];
  const green = 'rgb(0, 128, 0)';
  const applied = name => getComputedStyle(document.getElementById(`${name}-target`)).color === green;
  const styles = names.map(name => document.getElementById(name));
  const initial = {
    applied: names.map(applied),
    sheets: styles.map(style => style.sheet !== null),
    count: document.styleSheets.length,
    reflected: styles.map(style => style.type)
  };

  const spaced = document.getElementById('spaced');
  spaced.type = 'text/css';
  const valid = [applied('spaced'), spaced.sheet !== null, document.styleSheets.length];
  spaced.type = ' text/css ';
  const invalid = [applied('spaced'), spaced.sheet === null, document.styleSheets.length];
  return JSON.stringify({ initial, valid, invalid });
})()
"#,
        )
        .expect("style type attribute semantics should evaluate");

    assert_eq!(
        result,
        r#"{"initial":{"applied":[true,true,true,false,false],"sheets":[true,true,true,false,false],"count":3,"reflected":["","","TeXt/CsS"," text/css ","text/css; charset=utf-8"]},"valid":[true,true,4],"invalid":[false,true,3]}"#
    );
}

#[test]
fn style_observation_reuses_current_world_without_materializing_an_update() {
    let mut vm = new_parsed_test_vm(
        "https://style-observation-current-world.test/",
        "<!doctype html><style>.target { color: rgb(1, 2, 3); }</style><div id=target class=target></div>",
    );
    assert_eq!(
        vm.eval("getComputedStyle(document.getElementById('target')).color")
            .expect("initial style should establish the retained world"),
        "rgb(1, 2, 3)"
    );
    let target = element_handle_by_id(&vm, "target");
    let host = vm._context_host.borrow();
    let update_materializations = host.style_world_update_materializations_for_test();
    let full_snapshots = host.style_world_full_snapshots_for_test();

    let mut observation = crate::native_bridge::element::StyleObservation::new(&host);
    let first = observation
        .read(target)
        .computed_values()
        .expect("connected target should have computed values");
    let second = observation
        .read(target)
        .computed_values()
        .expect("a repeated read should retain computed values");

    assert!(style::servo_arc::Arc::ptr_eq(&first, &second));
    assert_eq!(
        host.style_world_update_materializations_for_test(),
        update_materializations
    );
    assert_eq!(host.style_world_full_snapshots_for_test(), full_snapshots);
}

#[test]
fn accessibility_style_batches_reuse_the_retained_world() {
    let mut vm = new_parsed_test_vm(
        "https://ax-style-batch.test/",
        "<!doctype html><div id=host><button>Action</button></div>",
    );
    vm.eval("host.attachShadow({mode:'closed'}).innerHTML='<button>Shadow action</button>'")
        .expect("closed shadow fixture");
    let handles = vm
        .document_runtime
        .dom_host()
        .nodes()
        .iter()
        .map(|node| node.id())
        .collect::<Vec<_>>();
    vm.accessibility_input(&handles).expect("initial AX styles");
    let host = vm._context_host.borrow();
    let updates = host.style_world_update_materializations_for_test();
    let full = host.style_world_full_snapshots_for_test();
    let environments = host.style_observation_environment_resolutions_for_test();
    drop(host);
    for _ in 0..2 {
        vm.accessibility_input(&handles).expect("warm AX styles");
    }
    let host = vm._context_host.borrow();
    assert_eq!(host.style_world_update_materializations_for_test(), updates);
    assert_eq!(host.style_world_full_snapshots_for_test(), full);
    assert_eq!(
        host.style_observation_environment_resolutions_for_test(),
        environments + 2,
        "each synchronous AX batch resolves its document environment once"
    );
}

#[test]
fn style_observation_resolves_page_environment_once_for_all_element_reads() {
    let mut vm = new_parsed_test_vm(
        "https://style-observation-page-environment.test/",
        "<!doctype html><meta name=color-scheme content='dark light'><div id=first></div><div id=second></div>",
    );
    vm.eval("getComputedStyle(document.getElementById('first')).color")
        .expect("initial style should establish the retained world");
    let first = element_handle_by_id(&vm, "first");
    let second = element_handle_by_id(&vm, "second");
    let host = vm._context_host.borrow();
    let before = host.style_observation_environment_resolutions_for_test();

    {
        let mut observation = crate::native_bridge::element::StyleObservation::new(&host);
        assert!(observation.read(first).computed_values().is_some());
        assert!(observation.read(second).computed_values().is_some());
        assert!(observation.read(first).computed_values().is_some());
    }
    assert_eq!(
        host.style_observation_environment_resolutions_for_test() - before,
        1,
        "one synchronous observation must snapshot page-level media inputs once"
    );

    let mut next_observation = crate::native_bridge::element::StyleObservation::new(&host);
    assert!(next_observation.read(first).computed_values().is_some());
    assert_eq!(
        host.style_observation_environment_resolutions_for_test() - before,
        2,
        "a later observation must take a fresh page-level environment snapshot"
    );
}

#[test]
fn style_observation_refreshes_dirty_element_without_materializing_a_world_update() {
    let mut vm = new_parsed_test_vm(
        "https://style-observation-dirty-element.test/",
        "<!doctype html><div id=target style='color: rgb(1, 2, 3)'></div>",
    );
    assert_eq!(
        vm.eval("getComputedStyle(document.getElementById('target')).color")
            .expect("initial style should establish the retained world"),
        "rgb(1, 2, 3)"
    );
    let target = element_handle_by_id(&vm, "target");
    let before = {
        let host = vm._context_host.borrow();
        let mut observation = crate::native_bridge::element::StyleObservation::new(&host);
        observation
            .read(target)
            .computed_values()
            .expect("initial computed values")
    };
    vm.eval("document.getElementById('target').style.color = 'rgb(4, 5, 6)' ")
        .expect("inline mutation should complete");

    let host = vm._context_host.borrow();
    let update_materializations = host.style_world_update_materializations_for_test();
    let full_snapshots = host.style_world_full_snapshots_for_test();
    let mut observation = crate::native_bridge::element::StyleObservation::new(&host);
    let after = observation
        .read(target)
        .computed_values()
        .expect("dirty target should be recomputed");

    assert!(!style::servo_arc::Arc::ptr_eq(&before, &after));
    assert_eq!(
        host.style_world_update_materializations_for_test(),
        update_materializations
    );
    assert_eq!(host.style_world_full_snapshots_for_test(), full_snapshots);
}

#[test]
fn style_observation_materializes_dirty_scope_once_without_a_full_snapshot() {
    let mut vm = new_parsed_test_vm(
        "https://style-observation-dirty-source.test/",
        "<!doctype html><style id=sheet>.target { color: rgb(1, 2, 3); }</style><div id=target class=target></div>",
    );
    assert_eq!(
        vm.eval("getComputedStyle(document.getElementById('target')).color")
            .expect("initial style should establish the retained world"),
        "rgb(1, 2, 3)"
    );
    vm.eval("document.getElementById('sheet').textContent = '.target { color: rgb(4, 5, 6); }'")
        .expect("stylesheet mutation should complete");
    let target = element_handle_by_id(&vm, "target");
    let host = vm._context_host.borrow();
    let update_materializations = host.style_world_update_materializations_for_test();
    let full_snapshots = host.style_world_full_snapshots_for_test();
    let document_scopes = host.style_world_document_scope_materializations_for_test();
    let shadow_scopes = host.style_world_shadow_scope_materializations_for_test();
    let environment_resolutions = host.style_observation_environment_resolutions_for_test();

    let mut observation = crate::native_bridge::element::StyleObservation::new(&host);
    let first = observation
        .read(target)
        .computed_values()
        .expect("dirty stylesheet world should be refreshed");
    let second = observation
        .read(target)
        .computed_values()
        .expect("the refreshed world should be reused");

    assert!(style::servo_arc::Arc::ptr_eq(&first, &second));
    assert_eq!(
        host.style_world_update_materializations_for_test()
            .saturating_sub(update_materializations),
        1
    );
    assert_eq!(
        host.style_world_full_snapshots_for_test()
            .saturating_sub(full_snapshots),
        0,
        "an incremental scope update must not materialize a full style-world snapshot"
    );
    assert_eq!(
        host.style_world_document_scope_materializations_for_test(),
        document_scopes + 1
    );
    assert_eq!(
        host.style_world_shadow_scope_materializations_for_test(),
        shadow_scopes
    );
    assert_eq!(
        host.style_observation_environment_resolutions_for_test() - environment_resolutions,
        1,
        "a dirty world update must reuse its observation's page-level environment"
    );
}

#[test]
fn document_stylesheet_mutation_reprojects_only_the_dirty_source() {
    let mut vm = new_parsed_test_vm(
        "https://style-observation-dirty-source-id.test/",
        r#"<!doctype html>
          <style id=sheet-a>.a { color: rgb(1, 2, 3); }</style>
          <style id=sheet-b>.b { color: rgb(4, 5, 6); }</style>
          <div id=target-a class=a></div><div id=target-b class=b></div>"#,
    );
    assert_eq!(
        vm.eval(
            r#"JSON.stringify([
              getComputedStyle(document.getElementById('target-a')).color,
              getComputedStyle(document.getElementById('target-b')).color,
            ])"#,
        )
        .expect("both stylesheet sources should initialize"),
        r#"["rgb(1, 2, 3)","rgb(4, 5, 6)"]"#
    );
    crate::style_engine::reset_source_cascade_rebuild_count_for_test();

    assert_eq!(
        vm.eval(
            r#"
document.getElementById('sheet-a').textContent = '.a { color: rgb(7, 8, 9); }';
JSON.stringify([
  getComputedStyle(document.getElementById('target-a')).color,
  getComputedStyle(document.getElementById('target-b')).color,
]);
"#,
        )
        .expect("the dirty stylesheet should update"),
        r#"["rgb(7, 8, 9)","rgb(4, 5, 6)"]"#
    );
    assert_eq!(
        crate::style_engine::source_cascade_rebuild_count_for_test(),
        1,
        "a single source mutation must not rebuild unrelated source cascade data"
    );
}

#[test]
fn style_observation_reconciles_an_empty_shadow_scope_once_per_version_change() {
    let mut vm = new_parsed_test_vm(
        "https://style-observation-shadow-version.test/",
        "<!doctype html><style>#target { color: rgb(1, 2, 3); }</style><div id=shadow-host></div><div id=target></div>",
    );
    assert_eq!(
        vm.eval("getComputedStyle(document.getElementById('target')).color")
            .expect("initial style should establish the retained world"),
        "rgb(1, 2, 3)"
    );
    let update_materializations_before_shadow = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let document_scopes_before_shadow = vm
        ._context_host
        .borrow()
        .style_world_document_scope_materializations_for_test();
    let shadow_scopes_before_shadow = vm
        ._context_host
        .borrow()
        .style_world_shadow_scope_materializations_for_test();
    vm.eval("document.getElementById('shadow-host').attachShadow({ mode: 'open' })")
        .expect("an empty connected ShadowRoot should attach");
    assert_eq!(
        vm.eval("getComputedStyle(document.getElementById('target')).color")
            .expect("the first read after attachment should reconcile TreeScopes"),
        "rgb(1, 2, 3)"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_update_materializations_for_test(),
        update_materializations_before_shadow + 1,
        "one TreeScope version change must materialize one incremental update batch"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_document_scope_materializations_for_test(),
        document_scopes_before_shadow,
        "adding an empty ShadowRoot must not recollect document stylesheets"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_shadow_scope_materializations_for_test(),
        shadow_scopes_before_shadow + 1,
        "only the newly connected ShadowRoot should be materialized"
    );
    let update_materializations_after_reconciliation = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    assert_eq!(
        vm.eval("getComputedStyle(document.getElementById('target')).color")
            .expect("a clean read should reuse the reconciled TreeScope universe"),
        "rgb(1, 2, 3)"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_update_materializations_for_test(),
        update_materializations_after_reconciliation,
        "a clean observation must compare versions without recollecting TreeScope sources"
    );

    vm.eval("document.getElementById('shadow-host').remove()")
        .expect("the ShadowRoot host should disconnect");
    assert_eq!(
        vm.eval("getComputedStyle(document.getElementById('target')).color")
            .expect("the first read after disconnection should remove the stale TreeScope"),
        "rgb(1, 2, 3)"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_update_materializations_for_test(),
        update_materializations_after_reconciliation + 1,
        "disconnecting a ShadowRoot host must reconcile the TreeScope universe once"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_shadow_scope_materializations_for_test(),
        shadow_scopes_before_shadow + 1,
        "removing a scope needs only its retained identity, not a source snapshot"
    );
}

#[test]
fn shadow_stylesheet_mutation_materializes_only_the_dirty_scope() {
    let mut vm = new_parsed_test_vm(
        "https://style-observation-dirty-shadow.test/",
        "<!doctype html><div id=host-a></div><div id=host-b></div>",
    );
    assert_eq!(
        vm.eval(
            r#"
const rootA = document.getElementById('host-a').attachShadow({ mode: 'open' });
rootA.innerHTML = '<style id=sheet>.target { color: rgb(1, 2, 3); }</style><span class=target></span>';
const rootB = document.getElementById('host-b').attachShadow({ mode: 'open' });
rootB.innerHTML = '<style>.target { color: rgb(4, 5, 6); }</style><span class=target></span>';
globalThis.__dirtyScopeRootA = rootA;
globalThis.__dirtyScopeRootB = rootB;
JSON.stringify([
  getComputedStyle(rootA.querySelector('.target')).color,
  getComputedStyle(rootB.querySelector('.target')).color,
]);
"#,
        )
        .expect("two ShadowRoot style worlds should initialize"),
        r#"["rgb(1, 2, 3)","rgb(4, 5, 6)"]"#
    );
    let host = vm._context_host.borrow();
    let document_scopes = host.style_world_document_scope_materializations_for_test();
    let shadow_scopes = host.style_world_shadow_scope_materializations_for_test();
    drop(host);

    assert_eq!(
        vm.eval(
            r#"
__dirtyScopeRootA.getElementById('sheet').textContent =
  '.target { color: rgb(7, 8, 9); }';
JSON.stringify([
  getComputedStyle(__dirtyScopeRootA.querySelector('.target')).color,
  getComputedStyle(__dirtyScopeRootB.querySelector('.target')).color,
]);
"#,
        )
        .expect("the dirty ShadowRoot should update"),
        r#"["rgb(7, 8, 9)","rgb(4, 5, 6)"]"#
    );
    let host = vm._context_host.borrow();
    assert_eq!(
        host.style_world_document_scope_materializations_for_test(),
        document_scopes,
        "a ShadowRoot mutation must not recollect document stylesheets"
    );
    assert_eq!(
        host.style_world_shadow_scope_materializations_for_test(),
        shadow_scopes + 1,
        "Shadow A must update without materializing Shadow B"
    );
}

#[test]
fn stylesheet_resource_manifest_reuses_current_world_and_tracks_revisions() {
    let mut vm = new_storage_test_vm("https://style-resource-manifest.test/document.html");
    vm.eval(
        r#"
const style = document.createElement('style');
style.id = 'resource-sheet';
style.textContent = '@font-face { font-family: First; src: url(font-a.woff2); }';
(document.head || document.documentElement || document).appendChild(style);
"#,
    )
    .expect("resource stylesheet fixture should initialize");
    let root = vm
        ._context_host
        .borrow()
        .dom_host()
        .document_element_handle()
        .expect("fixture should retain a document element");

    let first = crate::layout_renderer::current_native_stylesheet_resources(
        &vm._context_host.borrow(),
        root,
    )
    .expect("the first observation should publish a resource manifest");
    assert_eq!(first.web_fonts().len(), 1);
    assert_eq!(
        first.web_fonts()[0].request_url().as_str(),
        "https://style-resource-manifest.test/font-a.woff2"
    );
    let update_materializations_after_first = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();

    let clean = crate::layout_renderer::current_native_stylesheet_resources(
        &vm._context_host.borrow(),
        root,
    )
    .expect("a clean observation should reuse the resource manifest");
    assert_eq!(clean.generation(), first.generation());
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_update_materializations_for_test(),
        update_materializations_after_first,
        "a clean resource observation must not materialize a style-world update"
    );

    vm.eval(
        "document.getElementById('resource-sheet').textContent = \
         '@font-face { font-family: Second; src: url(font-b.woff2); }'",
    )
    .expect("resource stylesheet should mutate");
    let revised = crate::layout_renderer::current_native_stylesheet_resources(
        &vm._context_host.borrow(),
        root,
    )
    .expect("the revised observation should publish a resource manifest");
    assert_ne!(revised.generation(), first.generation());
    assert_eq!(revised.web_fonts().len(), 1);
    assert_eq!(
        revised.web_fonts()[0].request_url().as_str(),
        "https://style-resource-manifest.test/font-b.woff2"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_update_materializations_for_test(),
        update_materializations_after_first + 1,
        "one stylesheet revision should materialize one incremental world update"
    );
}

#[test]
fn disconnected_element_computed_style_returns_empty_without_caching() {
    let mut vm = new_storage_test_vm("https://disconnected-computed-style.test/page.html");
    let document = vm.document_handle_for_test();
    let initial_cache_count = vm.computed_style_cache_entry_count_for_document_for_test(document);

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.textContent = 'div { color: rgb(9, 10, 11); --active-token: active; }';
  head.appendChild(style);

  const detached = document.createElement('div');
  detached.setAttribute('style', 'color: rgb(1, 2, 3); background-image: url(./local.png)');
  const computed = getComputedStyle(detached);
  return JSON.stringify({
    color: computed.getPropertyValue('color'),
    display: computed.getPropertyValue('display'),
    backgroundImage: computed.getPropertyValue('background-image'),
    custom: computed.getPropertyValue('--active-token'),
    length: computed.length,
    cssText: computed.cssText
  });
})()
"#,
        )
        .expect("disconnected computed style should evaluate");

    assert_eq!(
        result,
        r#"{"color":"","display":"","backgroundImage":"","custom":"","length":0,"cssText":""}"#
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        initial_cache_count
    );
}

#[test]
fn detached_document_computed_style_does_not_use_active_document_context() {
    let mut vm = new_storage_test_vm("https://detached-doc-computed-style.test/page.html");
    let document = vm.document_handle_for_test();
    let initial_cache_count = vm.computed_style_cache_entry_count_for_document_for_test(document);

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const base = document.createElement('base');
  base.href = 'https://detached-doc-computed-style.test/active-base/';
  head.appendChild(base);
  const style = document.createElement('style');
  style.textContent = '#probe { color: rgb(7, 8, 9); background-image: url(active.png); --active-token: active; }';
  head.appendChild(style);

  const detachedDoc = document.implementation.createHTMLDocument('');
  const target = detachedDoc.createElement('div');
  target.id = 'probe';
  target.setAttribute('style', 'color: rgb(1, 2, 3); background-image: url(detached.png); --local-token: local;');
  detachedDoc.body.appendChild(target);
  const computed = getComputedStyle(target);
  return JSON.stringify({
    color: computed.getPropertyValue('color'),
    backgroundImage: computed.getPropertyValue('background-image'),
    activeCustom: computed.getPropertyValue('--active-token'),
    localCustom: computed.getPropertyValue('--local-token'),
    length: computed.length
  });
})()
"#,
        )
        .expect("detached document computed style should evaluate");

    assert_eq!(
        result,
        r#"{"color":"","backgroundImage":"","activeCustom":"","localCustom":"","length":0}"#
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        initial_cache_count
    );
}

#[test]
fn adopted_detached_document_node_reenters_and_leaves_active_style_context() {
    let mut vm = new_storage_test_vm("https://adopted-detached-computed-style.test/page.html");
    let document = vm.document_handle_for_test();

    let before = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#adopted-target { color: rgb(30, 31, 32); --active-token: active; }';
  head.appendChild(style);

  globalThis.__adoptedDetachedDocument = document.implementation.createHTMLDocument('');
  globalThis.__adoptedDetachedTarget = globalThis.__adoptedDetachedDocument.createElement('div');
  globalThis.__adoptedDetachedTarget.id = 'adopted-target';
  globalThis.__adoptedDetachedTarget.setAttribute('style', '--local-token: local;');
  globalThis.__adoptedDetachedDocument.body.appendChild(globalThis.__adoptedDetachedTarget);
  const detached = getComputedStyle(globalThis.__adoptedDetachedTarget);
  const detachedColor = detached.color;
  const detachedLength = detached.length;

  document.adoptNode(globalThis.__adoptedDetachedTarget);
  body.appendChild(globalThis.__adoptedDetachedTarget);
  const active = getComputedStyle(globalThis.__adoptedDetachedTarget);
  globalThis.__adoptedDetachedHeldStyle = active;
  return JSON.stringify({
    detachedColor,
    detachedLength,
    activeColor: active.color,
    activeCustom: active.getPropertyValue('--active-token'),
    ownerIsActive: globalThis.__adoptedDetachedTarget.ownerDocument === document
  });
})()
"#,
        )
        .expect("adopted detached computed style setup should evaluate");

    assert_eq!(
        before,
        r#"{"detachedColor":"","detachedLength":0,"activeColor":"rgb(30, 31, 32)","activeCustom":"active","ownerIsActive":true}"#
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let after = vm
        .eval(
            r#"
(() => {
  globalThis.__adoptedDetachedDocument.adoptNode(globalThis.__adoptedDetachedTarget);
  globalThis.__adoptedDetachedDocument.body.appendChild(globalThis.__adoptedDetachedTarget);
  const detached = getComputedStyle(globalThis.__adoptedDetachedTarget);
  const held = globalThis.__adoptedDetachedHeldStyle;
  const ownerIsDetached =
    globalThis.__adoptedDetachedTarget.ownerDocument === globalThis.__adoptedDetachedDocument;
  delete globalThis.__adoptedDetachedTarget;
  delete globalThis.__adoptedDetachedDocument;
  delete globalThis.__adoptedDetachedHeldStyle;
  return JSON.stringify({
    color: detached.color,
    activeCustom: detached.getPropertyValue('--active-token'),
    localCustom: detached.getPropertyValue('--local-token'),
    length: detached.length,
    heldColor: held.color,
    heldLength: held.length,
    ownerIsDetached
  });
})()
"#,
        )
        .expect("adopted node returning to detached document should evaluate");

    assert_eq!(
        after,
        r#"{"color":"","activeCustom":"","localCustom":"","length":0,"heldColor":"","heldLength":0,"ownerIsDetached":true}"#
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}

#[test]
fn removed_element_computed_style_is_empty_until_reattached() {
    let mut vm = new_storage_test_vm("https://removed-computed-style-cache.test/page.html");
    let document = vm.document_handle_for_test();

    let connected = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#target { color: rgb(11, 12, 13); }';
  head.appendChild(style);
  globalThis.__removedStyleTarget = document.createElement('div');
  globalThis.__removedStyleTarget.id = 'target';
  body.appendChild(globalThis.__removedStyleTarget);
  return getComputedStyle(globalThis.__removedStyleTarget).color;
})()
"#,
        )
        .expect("removed computed style setup should evaluate");

    assert_eq!(connected, "rgb(11, 12, 13)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let removed = vm
        .eval(
            r#"
(() => {
  globalThis.__removedStyleTarget.remove();
  const computed = getComputedStyle(globalThis.__removedStyleTarget);
  return `${computed.color}|${computed.length}`;
})()
"#,
        )
        .expect("removed computed style should evaluate");

    assert_eq!(removed, "|0");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let reattached = vm
        .eval(
            r#"
(() => {
  (document.body || document.documentElement || document).appendChild(globalThis.__removedStyleTarget);
  const value = getComputedStyle(globalThis.__removedStyleTarget).color;
  delete globalThis.__removedStyleTarget;
  return value;
})()
"#,
        )
        .expect("reattached computed style should evaluate");

    assert_eq!(reattached, "rgb(11, 12, 13)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
}

#[test]
fn get_computed_style_wrapper_creation_drains_pending_style_invalidations() {
    let mut vm = new_storage_test_vm("https://computed-style-wrapper-drain.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(10, 20, 30); }';
  head.appendChild(style);
  globalThis.__wrapperDrainTarget = document.createElement('div');
  body.appendChild(globalThis.__wrapperDrainTarget);
  return getComputedStyle(globalThis.__wrapperDrainTarget).color;
})()
"#,
        )
        .expect("computed style wrapper drain setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let wrapped = vm
        .eval(
            r#"
(() => {
  globalThis.__wrapperDrainTarget.setAttribute('class', 'active');
  globalThis.__wrapperDrainComputed = getComputedStyle(globalThis.__wrapperDrainTarget);
  return 'wrapped';
})()
"#,
        )
        .expect("computed style wrapper creation should evaluate");

    assert_eq!(wrapped, "wrapped");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let resolved = vm
        .eval(
            r#"
(() => {
  const color = globalThis.__wrapperDrainComputed.color;
  delete globalThis.__wrapperDrainComputed;
  delete globalThis.__wrapperDrainTarget;
  return color;
})()
"#,
        )
        .expect("held computed style should resolve after wrapper drain");

    assert_eq!(resolved, "rgb(10, 20, 30)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
}

#[test]
fn runtime_eval_turn_drains_pending_style_invalidations_without_computed_style_read() {
    let mut vm = new_storage_test_vm("https://runtime-evaluate-style-invalidation-drain.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(40, 50, 60); }';
  head.appendChild(style);
  globalThis.__runtimeEvalDrainTarget = document.createElement('div');
  body.appendChild(globalThis.__runtimeEvalDrainTarget);
  return getComputedStyle(globalThis.__runtimeEvalDrainTarget).color;
})()
"#,
        )
        .expect("runtime evaluate style drain setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let mutated = vm
        .eval(
            r#"
(() => {
  globalThis.__runtimeEvalDrainTarget.className = 'active';
  return 'mutated';
})()
"#,
        )
        .expect("runtime evaluate style mutation should evaluate");

    assert_eq!(mutated, "mutated");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let resolved = vm
        .eval(
            r#"
(() => {
  const color = getComputedStyle(globalThis.__runtimeEvalDrainTarget).color;
  delete globalThis.__runtimeEvalDrainTarget;
  return color;
})()
"#,
        )
        .expect("runtime evaluate drained style should resolve");

    assert_eq!(resolved, "rgb(40, 50, 60)");
}

#[test]
fn isolated_runtime_eval_turn_drains_pending_style_invalidations_without_computed_style_read() {
    let mut vm =
        new_storage_test_vm("https://isolated-runtime-evaluate-style-invalidation-drain.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(50, 60, 70); }';
  head.appendChild(style);
  const target = document.createElement('div');
  target.id = 'isolated-runtime-eval-drain-target';
  body.appendChild(target);
  return getComputedStyle(target).color;
})()
"#,
        )
        .expect("isolated runtime evaluate style drain setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let context_id = vm
        .create_isolated_world("style-drain-test", false)
        .expect("isolated world should be created");
    let mutated = vm
        .eval_in_isolated_context(
            context_id,
            r#"
(() => {
  document.getElementById('isolated-runtime-eval-drain-target').className = 'active';
  return 'mutated';
})()
"#,
        )
        .expect("isolated runtime evaluate style mutation should evaluate");

    assert_eq!(mutated, "mutated");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}

#[test]
fn isolated_runtime_exec_turn_drains_pending_style_invalidations_without_computed_style_read() {
    let mut vm =
        new_storage_test_vm("https://isolated-runtime-exec-style-invalidation-drain.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(70, 80, 90); }';
  head.appendChild(style);
  const target = document.createElement('div');
  target.id = 'isolated-runtime-exec-drain-target';
  body.appendChild(target);
  return getComputedStyle(target).color;
})()
"#,
        )
        .expect("isolated runtime exec style drain setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let context_id = vm
        .create_isolated_world("style-exec-drain-test", false)
        .expect("isolated world should be created");
    vm.exec_in_execution_context(
        context_id,
        r#"
document.getElementById('isolated-runtime-exec-drain-target').className = 'active';
"#,
    )
    .expect("isolated runtime exec style mutation should execute");

    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}

#[test]
fn document_start_run_immediately_drains_pending_style_invalidations_without_computed_style_read() {
    let mut vm = new_storage_test_vm("https://document-start-run-immediately-style-drain.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(80, 90, 100); }';
  head.appendChild(style);
  globalThis.__runImmediatelyDrainTarget = document.createElement('div');
  body.appendChild(globalThis.__runImmediatelyDrainTarget);
  return getComputedStyle(globalThis.__runImmediatelyDrainTarget).color;
})()
"#,
        )
        .expect("run-immediately style drain setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let result = vm
        .run_document_start_script_now(&crate::DocumentStartScript {
            registry_key: None,
            devtools_session: None,
            source: "globalThis.__runImmediatelyDrainTarget.className = 'active';".to_owned(),
            world_name: None,
            has_bidi_channel_argument: false,
            bidi_channel_handoffs: Vec::new(),
        })
        .expect("run-immediately document-start script should execute");
    assert_eq!(result, None);

    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}

#[test]
fn child_default_held_computed_style_reflects_class_mutation() {
    let mut vm = new_storage_test_vm("https://child-held-computed-probe.test/");

    let created = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const frame = document.createElement('iframe');
  frame.id = 'child-held-style-frame';
  body.appendChild(frame);
  return 'created';
})()
"#,
        )
        .expect("child frame setup should evaluate");
    assert_eq!(created, "created");

    vm.drain_pending_child_frame_work_for_test();
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child held computed-style setup");
    let child_document = child_document_handle_for_frame_id(&vm, "child-held-style-frame");

    let initial = vm
        .eval_in_child_default_context(
            child_context_id,
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(60, 70, 80); }';
  head.appendChild(style);
  globalThis.__childHeldStyleTarget = document.createElement('div');
  body.appendChild(globalThis.__childHeldStyleTarget);
  globalThis.__childHeldStyle = getComputedStyle(globalThis.__childHeldStyleTarget);
  return globalThis.__childHeldStyle.color;
})()
"#,
        )
        .expect("child held style setup should evaluate");
    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, child_document),
        1
    );

    let mutated = vm
        .eval_in_child_default_context(
            child_context_id,
            r#"
(() => {
  globalThis.__childHeldStyleTarget.className = 'active';
  return 'mutated';
})()
"#,
        )
        .expect("child held style mutation should evaluate");
    assert_eq!(mutated, "mutated");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, child_document),
        0
    );

    let after = vm
        .eval_in_child_default_context(
            child_context_id,
            r#"
(() => globalThis.__childHeldStyle.color)()
"#,
        )
        .expect("child held style readback should evaluate");
    assert_eq!(after, "rgb(60, 70, 80)");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, child_document),
        1
    );
}

#[tokio::test]
async fn host_task_turn_drains_pending_style_invalidations_without_computed_style_read() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader(
        "https://host-task-style-invalidation-drain.test/",
        &loader,
    );
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(20, 30, 40); }';
  head.appendChild(style);
  globalThis.__hostTaskDrainTarget = document.createElement('div');
  body.appendChild(globalThis.__hostTaskDrainTarget);
  return getComputedStyle(globalThis.__hostTaskDrainTarget).color;
})()
"#,
        )
        .expect("host task style drain setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    vm.eval(
        r#"
setTimeout(() => {
  globalThis.__hostTaskDrainTarget.className = 'active';
}, 0);
'queued';
"#,
    )
    .expect("host task style mutation should queue");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "the inline setup stylesheet queues its own event body before the timer"
    );
    assert!(
        vm.run_next_due_timer_callback_for_test(&loader)
            .await
            .expect("exact timer task should run")
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let resolved = vm
        .eval(
            r#"
(() => {
  const color = getComputedStyle(globalThis.__hostTaskDrainTarget).color;
  delete globalThis.__hostTaskDrainTarget;
  return color;
})()
"#,
        )
        .expect("host task drained style should resolve");

    assert_eq!(resolved, "rgb(20, 30, 40)");
}

#[test]
fn computed_color_resolves_simple_custom_property_fallbacks() {
    let mut vm = new_storage_test_vm("https://style-color-custom-property.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const target = document.createElement('span');
  root.append(target);
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('span { color: var(--color, red); }');
  document.adoptedStyleSheets = [sheet];
  const fallback = getComputedStyle(target).color;
  sheet.rules[0].style.setProperty('--color', 'green');
  const resolved = getComputedStyle(target).color;
  return [fallback, resolved].join('|');
})()
"#,
        )
        .expect("computed color custom property fallback should evaluate");

    assert_eq!(result, "rgb(255, 0, 0)|rgb(0, 128, 0)");
}

#[test]
fn computed_style_supports_revert_rule_keyword() {
    let mut vm = new_storage_test_vm("https://revert-rule-computed-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    #rule-basic { color: green; }
    #rule-basic { color: red; color: revert-rule; }
    #inline-basic { color: red; }
    #inline-basic { color: green; }
    #z-index-basic { z-index: 1; }
    #z-index-basic { z-index: 2; }
    #z-index-basic { z-index: -1; z-index: revert-rule; }
    #z-index-basic { z-index: -1; z-index: revert-rule; }
    #custom-basic { --a: red; --b: green; }
    #custom-basic { --a: green; --b: revert-rule; }
    #custom-basic { --a: revert-rule; --b: revert-rule; }
  `;
  head.appendChild(style);

  const ruleBasic = document.createElement('div');
  ruleBasic.id = 'rule-basic';
  const inlineBasic = document.createElement('div');
  inlineBasic.id = 'inline-basic';
  inlineBasic.setAttribute('style', 'color:red; color:revert-rule');
  const zIndexBasic = document.createElement('div');
  zIndexBasic.id = 'z-index-basic';
  const customBasic = document.createElement('div');
  customBasic.id = 'custom-basic';
  body.append(ruleBasic, inlineBasic, zIndexBasic, customBasic);

  const customStyle = getComputedStyle(customBasic);
  return [
    CSS.supports('color:revert-rule'),
    CSS.supports('z-index:revert-rule'),
    getComputedStyle(ruleBasic).color,
    getComputedStyle(inlineBasic).color,
    getComputedStyle(zIndexBasic).zIndex,
    customStyle.getPropertyValue('--a'),
    customStyle.getPropertyValue('--b')
  ].join('|');
})()
"#,
        )
        .expect("revert-rule computed style should evaluate");

    assert_eq!(
        result,
        "true|true|rgb(0, 128, 0)|rgb(0, 128, 0)|2|green|green"
    );
}

#[test]
fn focus_without_focus_selectors_preserves_computed_style_cache() {
    let mut vm = new_storage_test_vm("https://focus-style-cache-no-selector.test/");
    let document = vm.document_handle_for_test();

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#outside { color: rgb(1, 2, 3); } #target { color: rgb(4, 5, 6); }';
  head.appendChild(style);

  const outside = document.createElement('div');
  outside.id = 'outside';
  const target = document.createElement('button');
  target.id = 'target';
  body.append(outside, target);
  globalThis.__focusNoSelectorTarget = target;

  return [
    getComputedStyle(outside).color,
    getComputedStyle(target).color
  ].join('|');
})()
"#,
        )
        .expect("focus no-selector style setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(4, 5, 6)");
    let generation_before_focus =
        vm.computed_style_cache_generation_for_document_for_test(document);
    let cache_count_before_focus =
        vm.computed_style_cache_entry_count_for_document_for_test(document);

    let focused = vm
        .eval(
            r#"
(() => {
  globalThis.__focusNoSelectorTarget.focus();
  const active = document.activeElement === globalThis.__focusNoSelectorTarget;
  delete globalThis.__focusNoSelectorTarget;
  return String(active);
})()
"#,
        )
        .expect("focus no-selector mutation should evaluate");

    assert_eq!(focused, "true");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_before_focus
    );
    let cache_count_after_focus =
        vm.computed_style_cache_entry_count_for_document_for_test(document);
    assert!(
        cache_count_after_focus >= cache_count_before_focus,
        "focus without author focus selectors should not clear existing computed cache entries"
    );
}

#[test]
fn removing_focused_subtree_clears_focus_within_computed_style() {
    let mut vm = new_storage_test_vm("https://focus-within-remove-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  root.id = 'html';
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.id = 'body';

  const style = document.createElement('style');
  style.textContent = [
    '* { background-color: white; }',
    ':focus-within { background-color: rgb(1, 2, 3); }'
  ].join('\n');
  head.appendChild(style);

  const test = document.createElement('div');
  test.id = 'test';
  const container = document.createElement('div');
  container.id = 'container1';
  const sibling = document.createElement('div');
  sibling.id = 'sibling2';
  const target = document.createElement('input');
  target.id = 'target1';
  sibling.appendChild(target);
  container.appendChild(sibling);
  test.appendChild(container);
  body.appendChild(test);

  const styled = () => Array.from(document.querySelectorAll('*'))
    .filter((element) => getComputedStyle(element).backgroundColor === 'rgb(1, 2, 3)')
    .map((element) => element.id)
    .join(',');
  const matched = () => Array.from(document.querySelectorAll(':focus-within'))
    .map((element) => element.id)
    .join(',');

  target.focus();
  const before = `${styled()}|${matched()}|${target.matches(':focus')}`;
  container.remove();
  const afterRemove = `${styled()}|${matched()}|${container.querySelectorAll(':focus-within').length}|${target.matches(':focus')}`;
  target.focus();
  const afterDetachedFocus = `${styled()}|${matched()}|${container.querySelectorAll(':focus-within').length}|${target.matches(':focus')}`;
  return `${before}\n${afterRemove}\n${afterDetachedFocus}`;
})()
"#,
        )
        .expect("focus-within subtree removal should evaluate");

    assert_eq!(
        result,
        "html,body,test,container1,sibling2,target1|html,body,test,container1,sibling2,target1|true\n\
||0|false\n\
||0|false"
    );
}

#[test]
fn focus_dependent_selector_invalidates_computed_style_cache() {
    let mut vm = new_storage_test_vm("https://focus-style-cache-selector.test/");
    let document = vm.document_handle_for_test();

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = [
    '#sibling { color: rgb(1, 2, 3); }',
    '#target:focus + #sibling { color: rgb(4, 5, 6); }'
  ].join('\n');
  head.appendChild(style);

  const target = document.createElement('button');
  target.id = 'target';
  const sibling = document.createElement('div');
  sibling.id = 'sibling';
  body.append(target, sibling);
  globalThis.__focusSelectorTarget = target;
  globalThis.__focusSelectorSiblingStyle = getComputedStyle(sibling);
  return globalThis.__focusSelectorSiblingStyle.color;
})()
"#,
        )
        .expect("focus selector style setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)");
    let generation_before_focus =
        vm.computed_style_cache_generation_for_document_for_test(document);

    let focused = vm
        .eval(
            r#"
(() => {
  globalThis.__focusSelectorTarget.focus();
  const color = globalThis.__focusSelectorSiblingStyle.color;
  delete globalThis.__focusSelectorTarget;
  delete globalThis.__focusSelectorSiblingStyle;
  return color;
})()
"#,
        )
        .expect("focus selector mutation should evaluate");

    assert_eq!(focused, "rgb(4, 5, 6)");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_before_focus,
        "targeted focus invalidation should not bump the retained style generation"
    );
}
