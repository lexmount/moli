use super::*;

#[test]
fn computed_style_index_enumeration_does_not_rematerialize_world_per_standard_property() {
    let mut vm = new_storage_test_vm("https://computed-style-index-cost.test/");
    vm.eval(
        r#"
const target = document.createElement('div');
target.id = 'target';
(document.body || document.documentElement || document).appendChild(target);
globalThis.__computedStyleForIndexCost = getComputedStyle(target);
"#,
    )
    .expect("computed-style fixture should initialize");
    let update_materializations_before = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();

    let length = vm
        .eval("String(__computedStyleForIndexCost.length)")
        .expect("computed style length should be readable");
    let after_length = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    vm.eval("String(__computedStyleForIndexCost[0])")
        .expect("first indexed computed property should be readable");
    let after_index = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    vm.eval("String(__computedStyleForIndexCost.item(0))")
        .expect("first computed style item should be readable");
    let after_item = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    vm.eval("String(0 in __computedStyleForIndexCost)")
        .expect("first computed style index should be queryable");
    let after_query = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let count = vm
        .eval("String(Array.from(__computedStyleForIndexCost).length)")
        .expect("computed properties should be enumerable")
        .parse::<u64>()
        .expect("computed property count");

    let update_materializations = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test()
        .saturating_sub(update_materializations_before);
    assert!(count >= 266, "unexpectedly narrow computed style: {count}");
    assert_eq!(
        after_length.saturating_sub(update_materializations_before),
        1
    );
    assert_eq!(after_index.saturating_sub(after_length), 0);
    assert_eq!(after_item.saturating_sub(after_index), 0);
    assert_eq!(after_query.saturating_sub(after_item), 0);
    assert!(
        update_materializations <= 8,
        "indexed enumeration materialized {update_materializations} style-world updates for {count} properties; \
         length={length}, deltas length/index/item/query={}/{}/{}/{}",
        after_length.saturating_sub(update_materializations_before),
        after_index.saturating_sub(after_length),
        after_item.saturating_sub(after_index),
        after_query.saturating_sub(after_item),
    );
}

#[test]
fn held_computed_style_count_cache_tracks_custom_property_mutations() {
    let mut vm = new_storage_test_vm("https://computed-style-custom-count-cache.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  target.style.setProperty('--before', 'one');
  (document.body || document.documentElement || document).appendChild(target);
  const style = getComputedStyle(target);
  const before = Array.from(style);
  target.style.removeProperty('--before');
  target.style.setProperty('--after', 'two');
  const after = Array.from(style);
  return JSON.stringify({
    beforeLength: before.length,
    afterLength: after.length,
    beforeHasBefore: before.includes('--before'),
    beforeHasAfter: before.includes('--after'),
    afterHasBefore: after.includes('--before'),
    afterHasAfter: after.includes('--after'),
    afterValue: style.getPropertyValue('--after'),
  });
})()
"#,
        )
        .expect("held computed style should track custom-property mutations");

    let result: serde_json::Value = serde_json::from_str(&result).expect("valid JSON summary");
    assert_eq!(result["beforeLength"], result["afterLength"]);
    assert_eq!(result["beforeHasBefore"], serde_json::json!(true));
    assert_eq!(result["beforeHasAfter"], serde_json::json!(false));
    assert_eq!(result["afterHasBefore"], serde_json::json!(false));
    assert_eq!(result["afterHasAfter"], serde_json::json!(true));
    assert_eq!(result["afterValue"], serde_json::json!("two"));
}

#[test]
fn inspector_computed_style_bulk_read_prepares_dirty_world_once_and_reuses_clean_world() {
    let mut vm = new_storage_test_vm("https://computed-style-inspector-bulk.test/");
    vm.eval(
        r#"
const style = document.createElement('style');
style.textContent = `#target {
  animation-timeline: auto;
  animation-range-start: entry 10%;
  animation-range-end: exit 20%;
  background-position-x: 25%;
  column-span: all;
  column-width: 12px;
  font-variant-alternates: historical-forms;
  font-variant-emoji: emoji;
  font-variant-position: super;
  grid-auto-columns: 17px;
  object-fit: cover;
  overflow-wrap: anywhere;
  pointer-events: none;
  white-space-collapse: preserve;
  zoom: 125%;
}`;
(document.head || document.documentElement || document).appendChild(style);
const target = document.createElement('div');
target.id = 'target';
target.style.setProperty('--inspector-token', 'present');
(document.body || document.documentElement || document).appendChild(target);
const unrelatedHost = document.createElement('div');
(document.body || document.documentElement || document).appendChild(unrelatedHost);
unrelatedHost.attachShadow({ mode: 'open' }).innerHTML =
  '<style>:host { color: rgb(1, 2, 3); }</style><span>unrelated</span>';
"#,
    )
    .expect("inspector computed-style fixture should initialize");
    let target = element_handle_by_id(&vm, "target");
    let update_materializations_before = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let property_reads_before = vm
        ._context_host
        .borrow()
        .stylo_computed_style_property_reads_for_test();

    let properties = vm
        .computed_style_properties_for_inspector_handle(target)
        .expect("live element should resolve");

    let update_materializations = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test()
        .saturating_sub(update_materializations_before);
    let property_reads = vm
        ._context_host
        .borrow()
        .stylo_computed_style_property_reads_for_test()
        .saturating_sub(property_reads_before);
    assert_eq!(
        update_materializations, 1,
        "one inspector bulk read must materialize one style-world update"
    );
    assert!(properties.len() >= 267);
    assert!(
        property_reads.saturating_mul(4) < properties.len() as u64,
        "bulk serialization re-entered Stylo {property_reads} times for {} projected properties",
        properties.len(),
    );
    let properties = properties
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
    for (name, expected) in [
        ("animation-timeline", "auto"),
        ("animation-range-start", "entry 10%"),
        ("animation-range-end", "exit 20%"),
        ("background-position-x", "25%"),
        ("column-span", "all"),
        ("column-width", "12px"),
        ("font-variant-alternates", "historical-forms"),
        ("font-variant-emoji", "emoji"),
        ("font-variant-position", "super"),
        ("grid-auto-columns", "17px"),
        ("object-fit", "cover"),
        ("overflow-wrap", "anywhere"),
        ("pointer-events", "none"),
        ("white-space-collapse", "preserve"),
        ("zoom", "1.25"),
        ("--inspector-token", "present"),
    ] {
        assert_eq!(properties.get(name).map(String::as_str), Some(expected));
    }

    let clean_update_materializations = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let clean_full_snapshots = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();
    let clean_properties = vm
        .computed_style_properties_for_inspector_handle(target)
        .expect("a clean retained world should remain readable");
    assert_eq!(clean_properties.len(), properties.len());
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_update_materializations_for_test(),
        clean_update_materializations,
        "a clean inspector bulk read must not materialize a style-world update"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_full_snapshots_for_test(),
        clean_full_snapshots,
        "a clean inspector bulk read must not materialize a full style-world snapshot"
    );
}

#[test]
fn inspector_computed_style_bulk_read_handles_large_inherited_custom_property_sets() {
    const CUSTOM_PROPERTY_COUNT: usize = 512;
    let mut vm = new_storage_test_vm("https://computed-style-inspector-many-custom.test/");
    vm.eval(&format!(
        r#"
CSS.registerProperty({{
  name: '--non-inherited-bulk',
  syntax: '*',
  inherits: false,
  initialValue: 'registered-initial'
}});
const style = document.createElement('style');
style.textContent = `:root {{${{Array.from(
  {{ length: {CUSTOM_PROPERTY_COUNT} }},
  (_, index) => `--bulk-${{String(index).padStart(4, '0')}}: value-${{index}};`
).join('')}}--removed-bulk: ancestor;}}
#target {{ --removed-bulk: initial; --non-inherited-bulk: registered-local; }}`;
(document.head || document.documentElement || document).appendChild(style);
const target = document.createElement('div');
target.id = 'target';
(document.body || document.documentElement || document).appendChild(target);
"#,
    ))
    .expect("large custom-property fixture should initialize");
    let target = element_handle_by_id(&vm, "target");

    for observation in 0..2 {
        let properties = vm
            .computed_style_properties_for_inspector_handle(target)
            .expect("live element should resolve");
        assert_eq!(
            properties
                .iter()
                .filter(|(name, _)| name.starts_with("--bulk-"))
                .count(),
            CUSTOM_PROPERTY_COUNT,
            "observation {observation} lost custom properties",
        );
        let properties = properties
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();
        for (name, expected) in [
            ("--bulk-0000", "value-0"),
            ("--bulk-0256", "value-256"),
            ("--bulk-0511", "value-511"),
            ("--non-inherited-bulk", "registered-local"),
        ] {
            assert_eq!(
                properties.get(name).map(String::as_str),
                Some(expected),
                "observation {observation} returned the wrong value for {name}",
            );
        }
        assert!(
            !properties.contains_key("--removed-bulk"),
            "observation {observation} exposed a tombstoned inherited value",
        );
    }
}

#[test]
fn inspector_computed_style_bulk_read_uses_child_document_scope_and_viewport() {
    let mut vm = new_storage_test_vm("https://computed-style-inspector-child.test/");
    vm.eval(
        r#"
const frame = document.createElement('iframe');
frame.id = 'inspector-child-frame';
frame.style.width = '200px';
frame.style.height = '100px';
(document.body || document.documentElement || document).appendChild(frame);
const childDocument = frame.contentWindow.document;
childDocument.open();
childDocument.write(`
  <style>
    #inspector-child-target {
      color: rgb(255, 0, 0);
      pointer-events: auto;
      width: 10px;
    }
    @media (width: 200px) {
      #inspector-child-target {
        color: rgb(0, 128, 0);
        pointer-events: none;
        width: 50vw;
        --child-inspector-token: child;
      }
    }
  </style>
  <body><div id="inspector-child-target"></div></body>`);
childDocument.close();
"#,
    )
    .expect("child inspector computed-style fixture should initialize");
    let target = element_handle_by_id(&vm, "inspector-child-target");

    let properties = vm
        .computed_style_properties_for_inspector_handle(target)
        .expect("child-frame element should resolve")
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
    for (name, expected) in [
        ("color", "rgb(0, 128, 0)"),
        ("pointer-events", "none"),
        ("width", "100px"),
        ("--child-inspector-token", "child"),
    ] {
        assert_eq!(properties.get(name).map(String::as_str), Some(expected));
    }

    vm.eval("document.getElementById('inspector-child-frame').style.display = 'none'")
        .expect("child frame should become hidden");
    assert_eq!(
        vm.computed_style_properties_for_inspector_handle(target),
        Some(Vec::new()),
        "an existing element in a hidden child frame has an empty computed declaration",
    );
}

#[test]
fn inspector_computed_style_bulk_read_reuses_clean_shadow_world() {
    let mut vm = new_storage_test_vm("https://computed-style-inspector-shadow.test/");
    vm.eval(
        r#"
const host = document.createElement('div');
(document.body || document.documentElement || document).appendChild(host);
host.attachShadow({ mode: 'open' }).innerHTML = `
  <style>
    #inspector-shadow-target {
      color: rgb(1, 2, 3);
      pointer-events: none;
      --shadow-inspector-token: shadow;
    }
  </style>
  <div id="inspector-shadow-target"></div>`;
"#,
    )
    .expect("shadow inspector computed-style fixture should initialize");
    let target = element_handle_by_id(&vm, "inspector-shadow-target");
    let update_materializations_before = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();

    let properties = vm
        .computed_style_properties_for_inspector_handle(target)
        .expect("shadow element should resolve")
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();

    let update_materializations = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test()
        .saturating_sub(update_materializations_before);
    assert_eq!(
        update_materializations, 1,
        "one shadow-target inspector bulk read must materialize one style-world update",
    );
    for (name, expected) in [
        ("color", "rgb(1, 2, 3)"),
        ("pointer-events", "none"),
        ("--shadow-inspector-token", "shadow"),
    ] {
        assert_eq!(properties.get(name).map(String::as_str), Some(expected));
    }

    let clean_update_materializations = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let clean_properties = vm
        .computed_style_properties_for_inspector_handle(target)
        .expect("clean shadow target should resolve");
    assert_eq!(clean_properties.len(), properties.len());
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_update_materializations_for_test(),
        clean_update_materializations,
        "a clean ShadowRoot observation must not rebuild TreeScope inputs"
    );
}

#[test]
fn computed_style_map_exposes_live_typed_values_and_readonly_map_shape() {
    let mut vm = new_storage_test_vm("https://computed-style-map.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  target.style.cssText = 'font-size: 21px; display: block';
  (document.body || document.documentElement || document).appendChild(target);

  const map = target.computedStyleMap();
  const first = map.get('font-size');
  const display = map.get('display');
  target.style.fontSize = '32px';
  const updated = map.get('FoNt-SiZe');
  let invalid = 'none';
  try {
    map.get('not-a-real-property');
  } catch (error) {
    invalid = error && error.name;
  }
  const constructed = new CSSUnitValue(2.5, 'PX');
  constructed.value = 3;
  const constructedKeyword = new CSSKeywordValue('inline');
  constructedKeyword.value = 'grid';
  let emptyKeyword = 'none';
  try {
    constructedKeyword.value = '';
  } catch (error) {
    emptyKeyword = error && error.name;
  }
  const entries = Array.from(map);
  const fontEntry = entries.find(([name]) => name === 'font-size');

  return JSON.stringify({
    sameMap: map === target.computedStyleMap(),
    mapBrand: map instanceof StylePropertyMapReadOnly,
    firstBrand: first instanceof CSSUnitValue &&
      first instanceof CSSNumericValue &&
      first instanceof CSSStyleValue,
    first: [first.value, first.unit, first.toString()],
    updated: updated.toString(),
    keywordBrand: display instanceof CSSKeywordValue &&
      display instanceof CSSStyleValue,
    keyword: [display.value, display.toString()],
    has: map.has('font-size'),
    missingCustom: map.get('--missing') === undefined,
    getAll: map.getAll('font-size').map(value => value.toString()),
    size: map.size >= 8,
    keys: Array.from(map.keys()).includes('font-size'),
    entry: fontEntry && [
      fontEntry[0],
      Array.isArray(fontEntry[1]),
      fontEntry[1][0].toString()
    ],
    invalid,
    constructed: [
      constructed.value,
      constructed.unit,
      constructed.toString(),
      Object.prototype.toString.call(constructed)
    ],
    constructedKeyword: [
      constructedKeyword.value,
      constructedKeyword.toString(),
      Object.prototype.toString.call(constructedKeyword),
      emptyKeyword
    ]
  });
})()
"#,
        )
        .expect("computed StylePropertyMap should expose typed computed values");

    assert_eq!(
        result,
        r#"{"sameMap":true,"mapBrand":true,"firstBrand":true,"first":[21,"px","21px"],"updated":"32px","keywordBrand":true,"keyword":["block","block"],"has":true,"missingCustom":true,"getAll":["32px"],"size":true,"keys":true,"entry":["font-size",true,"32px"],"invalid":"TypeError","constructed":[3,"px","3px","[object CSSUnitValue]"],"constructedKeyword":["grid","grid","[object CSSKeywordValue]","TypeError"]}"#
    );
}

#[test]
fn computed_style_map_uses_native_reads_and_validates_typed_om_calls() {
    let mut vm = new_storage_test_vm("https://computed-style-map-native.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  target.style.cssText =
    'transition-duration: 1s, 2s; --Token: exact; font-size: 18px';
  (document.body || document.documentElement || document).appendChild(target);
  const map = target.computedStyleMap();

  getComputedStyle = () => ({ getPropertyValue: () => 'wrong' });
  CSSStyleDeclaration.prototype.getPropertyValue = () => 'also-wrong';

  const errorName = callback => {
    try {
      callback();
      return 'none';
    } catch (error) {
      return error && error.name;
    }
  };
  const durations = map.getAll('transition-duration');
  const custom = map.get('--Token');
  target.remove();
  const removed = map.get('font-size');
  (document.body || document.documentElement || document).appendChild(target);

  return JSON.stringify({
    firstDuration: map.get('transition-duration').toString(),
    durations: durations.map(value => [
      value.toString(),
      value instanceof CSSUnitValue,
      value.unit
    ]),
    custom: custom && custom.toString(),
    customCaseSensitive: map.get('--token') === undefined,
    nativeRead: map.get('font-size').toString(),
    removed: removed === undefined,
    reattached: map.get('font-size').toString(),
    errors: [
      errorName(() => map.getAll('not-a-real-property')),
      errorName(() => map.has('not-a-real-property')),
      errorName(() => StylePropertyMapReadOnly.prototype.get.call({}, 'font-size')),
      errorName(() => new CSSUnitValue(0, 'lemon')),
      errorName(() => CSSUnitValue(1, 'px'))
    ]
  });
})()
"#,
        )
        .expect("computed StylePropertyMap should use native computed style reads");

    assert_eq!(
        result,
        r#"{"firstDuration":"1s","durations":[["1s",true,"s"],["2s",true,"s"]],"custom":"exact","customCaseSensitive":true,"nativeRead":"18px","removed":true,"reattached":"18px","errors":["TypeError","TypeError","TypeError","TypeError","TypeError"]}"#
    );
}

#[test]
fn computed_style_enumerates_registered_custom_properties() {
    let mut vm = new_storage_html_test_vm("https://computed-style-custom-properties.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  const outer = document.createElement('div');
  outer.id = 'outer';
  const innerNode = document.createElement('div');
  innerNode.id = 'inner';
  const siblingNode = document.createElement('div');
  siblingNode.id = 'sibling';
  outer.append(innerNode, siblingNode);
  root.appendChild(outer);
  const style = document.createElement('style');
  style.textContent = `
    @property --non-inherited-length {
      syntax: "<length>";
      inherits: false;
      initial-value: 0px;
    }
    @property --inherited-length {
      syntax: "<length>";
      inherits: true;
      initial-value: 0px;
    }
    @property --universal-without-initial {
      syntax: "*";
      inherits: false;
    }
    #outer { --non-registered-outer: 1px; }
    #inner { --non-registered-inner: 2px; }
    #sibling { --universal-without-initial: bar; }
  `;
  (document.head || root).appendChild(style);
  const inner = Array.from(getComputedStyle(document.getElementById('inner')));
  const sibling = Array.from(getComputedStyle(document.getElementById('sibling')));
  return JSON.stringify({
    innerRegistered: inner.includes('--non-inherited-length') && inner.includes('--inherited-length'),
    innerInherited: inner.includes('--non-registered-outer'),
    innerOwn: inner.includes('--non-registered-inner'),
    innerNoInitial: inner.includes('--universal-without-initial'),
    siblingInherited: sibling.includes('--non-registered-outer'),
    siblingOwnNoInitial: sibling.includes('--universal-without-initial'),
    siblingInnerAbsent: sibling.includes('--non-registered-inner')
  });
})()
"#,
        )
        .expect("computed style should enumerate registered custom properties");

    assert_eq!(
        result,
        r#"{"innerRegistered":true,"innerInherited":true,"innerOwn":true,"innerNoInitial":false,"siblingInherited":true,"siblingOwnNoInitial":true,"siblingInnerAbsent":false}"#
    );
}

#[test]
fn computed_style_property_names_place_custom_properties_after_longhands() {
    let mut vm = new_storage_test_vm("https://computed-style-custom-property-order.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.body || document.documentElement ||
    document.appendChild(document.createElement('html'));
  const target = document.createElement('div');
  const plain = document.createElement('div');
  target.style.cssText = '--z-token: z; --a-token: a;';
  root.append(target, plain);

  const computed = getComputedStyle(target);
  const names = Array.from(
    { length: computed.length },
    (_, index) => computed.item(index)
  );
  const customStart = names.indexOf('--a-token');
  const lastVendor = names.findLastIndex(
    name => name.startsWith('-') && !name.startsWith('--')
  );
  return [
    computed.getPropertyValue('--a-token'),
    computed.length - getComputedStyle(plain).length,
    names.slice(-2).join(','),
    computed.item(computed.length - 1),
    computed[computed.length - 1],
    customStart > lastVendor
  ].join('|');
})()
"#,
        )
        .expect("computed custom property order should evaluate");

    assert_eq!(result, "a|2|--a-token,--z-token|--z-token|--z-token|true");
}

#[test]
fn css_register_property_validates_and_updates_computed_style() {
    let mut vm = new_storage_test_vm("https://css-register-property.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  const target = document.createElement('div');
  target.id = 'target';
  target.style.cssText = '--registered-length: 12px; --registered-color: nope;';
  root.appendChild(target);

  const errorName = (fn) => {
    try {
      fn();
      return 'none';
    } catch (error) {
      return `${error && error.name}:${error instanceof DOMException}`;
    }
  };

  const errors = [
    errorName(() => CSS.registerProperty()),
    errorName(() => CSS.registerProperty({ name: '--missing-inherits' })),
    errorName(() => CSS.registerProperty({ name: 'no-leading-dash', inherits: false })),
    errorName(() => CSS.registerProperty({
      name: '--bad-syntax',
      syntax: '<banana>',
      initialValue: 'banana',
      inherits: false
    })),
    errorName(() => CSS.registerProperty({
      name: '--missing-initial',
      syntax: '<length>',
      inherits: false
    })),
    errorName(() => CSS.registerProperty({
      name: '--invalid-universal',
      syntax: '*',
      initialValue: 'semi;colon',
      inherits: false
    })),
    errorName(() => CSS.registerProperty({
      name: '--dependent-length',
      syntax: '<length>',
      initialValue: 'calc(4px + 3em)',
      inherits: false
    })),
    errorName(() => CSS.registerProperty({
      name: '--unitless-angle',
      syntax: '<angle>',
      initialValue: '0',
      inherits: false
    })),
    errorName(() => CSS.registerProperty({
      name: '--negative-resolution',
      syntax: '<resolution>',
      initialValue: '-5.3dpcm',
      inherits: false
    })),
    errorName(() => CSS.registerProperty({
      name: '--empty-transform',
      syntax: '<transform-function>',
      initialValue: 'scale()',
      inherits: false
    })),
    errorName(() => CSS.registerProperty({
      name: '--image-none',
      syntax: '<image>',
      initialValue: 'none',
      inherits: false
    }))
  ];

  CSS.registerProperty({
    name: '--registered-length',
    syntax: '<length>',
    initialValue: '4px',
    inherits: false
  });
  CSS.registerProperty({
    name: '--registered-color',
    syntax: '<color>',
    initialValue: 'red',
    inherits: false
  });
  CSS.registerProperty({
    name: '--registered-image-light-dark',
    syntax: '<image>',
    initialValue: 'light-dark(none, none)',
    inherits: false
  });
  const duplicate = errorName(() => CSS.registerProperty({
    name: '--registered-length',
    syntax: '<percentage>',
    initialValue: '0%',
    inherits: false
  }));
  const computed = getComputedStyle(target);
  return JSON.stringify({
    type: typeof CSS.registerProperty,
    length: CSS.registerProperty.length,
    errors,
    duplicate,
    registeredLength: computed.getPropertyValue('--registered-length'),
    registeredColor: computed.getPropertyValue('--registered-color')
  });
})()
"#,
        )
        .expect("CSS.registerProperty should validate and update computed style");

    assert_eq!(
        result,
        r#"{"type":"function","length":1,"errors":["TypeError:false","TypeError:false","SyntaxError:true","SyntaxError:true","SyntaxError:true","SyntaxError:true","SyntaxError:true","SyntaxError:true","SyntaxError:true","SyntaxError:true","SyntaxError:true"],"duplicate":"InvalidModificationError:true","registeredLength":"12px","registeredColor":"rgb(255, 0, 0)"}"#
    );
}

#[test]
fn child_frame_css_register_property_is_scoped_to_child_document_world() {
    let mut vm = new_storage_test_vm("https://child-register-property-scope.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));

  CSS.registerProperty({
    name: '--cross-doc-token',
    syntax: '<length>',
    initialValue: '9px',
    inherits: false
  });

  const active = document.createElement('div');
  active.id = 'active-register-target';
  active.style.cssText = '--cross-doc-token: rgb(1, 2, 3); width: var(--cross-doc-token);';
  body.appendChild(active);

  const frame = document.createElement('iframe');
  frame.id = 'register-child-frame';
  body.appendChild(frame);
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write('<body><div id="child-register-target" style="--cross-doc-token: 8px; color: var(--cross-doc-token)"></div></body>');
  childDocument.close();

  childWindow.CSS.registerProperty({
    name: '--cross-doc-token',
    syntax: '<color>',
    initialValue: 'rgb(4, 5, 6)',
    inherits: false
  });

  const childTarget = childDocument.getElementById('child-register-target');
  return [
    getComputedStyle(active).width,
    childWindow.getComputedStyle(childTarget).color,
    childWindow.CSS !== CSS
  ].join('|');
})()
"#,
        )
        .expect("child frame CSS.registerProperty should evaluate");

    assert_eq!(result, "9px|rgb(4, 5, 6)|true");

    let active_document = vm.document_runtime.dom_host().document_handle();
    let child_document = child_document_handle_for_frame_id(&vm, "register-child-frame");
    assert!(computed_style_cache_entry_count_for_document(&vm, active_document) > 0);
    assert!(computed_style_cache_entry_count_for_document(&vm, child_document) > 0);
}

#[test]
fn isolated_world_css_register_property_uses_root_document_world() {
    let mut vm = new_storage_test_vm("https://isolated-register-property-scope.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  target.id = 'isolated-register-target';
  target.style.cssText = '--isolated-token: 12px; color: var(--isolated-token);';
  body.appendChild(target);
  return getComputedStyle(target).color;
})()
"#,
        )
        .expect("isolated CSS.registerProperty setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    let cache_entries = computed_style_cache_entry_count_for_document(&vm, document);
    assert!(cache_entries > 0);
    let stylist_identity = vm.retained_stylist_identity_for_document_for_test(document);
    let rebuilds = vm.retained_style_system_rebuild_count_for_document_for_test(document);
    let updates = vm.retained_style_system_update_count_for_document_for_test(document);

    let context_id = vm
        .create_isolated_world("style-register-property-test", false)
        .expect("isolated world should be created");
    let registered = vm
        .eval_in_isolated_context(
            context_id,
            r#"
(() => {
  CSS.registerProperty({
    name: '--isolated-token',
    syntax: '<color>',
    initialValue: 'rgb(10, 20, 30)',
    inherits: false
  });
  return typeof CSS.registerProperty;
})()
"#,
        )
        .expect("isolated CSS.registerProperty should evaluate");

    assert_eq!(registered, "function");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, document),
        cache_entries,
        "registerProperty must defer invalidation until the next style observation"
    );
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(document),
        stylist_identity,
        "registerProperty must keep the existing document Stylist"
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(document),
        updates,
        "registerProperty must only mark the style world dirty"
    );

    let resolved = vm
        .eval(
            r#"
(() => getComputedStyle(document.getElementById('isolated-register-target')).color)()
"#,
        )
        .expect("default world computed style should use isolated registration");

    assert_eq!(resolved, "rgb(10, 20, 30)");
    assert!(computed_style_cache_entry_count_for_document(&vm, document) > 0);
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(document),
        stylist_identity
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(document),
        updates + 1,
        "the first post-registration observation must update the retained Stylist once"
    );
}

#[test]
fn popup_css_register_property_uses_popup_document_world() {
    let mut vm = new_storage_test_vm("https://popup-register-property-scope.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const active = document.createElement('div');
  active.id = 'active-popup-register-target';
  active.style.cssText = '--popup-token: 12px; color: var(--popup-token);';
  body.appendChild(active);

  const popup = open('about:blank');
  globalThis.__styleRegisterPopup = popup;
  const popupBody = popup.document.body || popup.document.documentElement || popup.document;
  const target = popup.document.createElement('div');
  target.id = 'popup-register-target';
  target.style.cssText = '--popup-token: 12px; color: var(--popup-token);';
  popupBody.appendChild(target);
  const plain = popup.document.createElement('div');
  plain.id = 'popup-plain-color-target';
  plain.style.cssText = 'color: rgb(7, 8, 9); width: 25%; height: 50%;';
  popupBody.appendChild(plain);

  return [
    getComputedStyle(active).color,
    popup.getComputedStyle(target).color,
    popup.getComputedStyle(plain).color,
    popup.getComputedStyle(plain).width,
    popup.getComputedStyle(plain).height,
    plain.getAttribute('style'),
    popup.CSS !== CSS
  ].join('|');
})()
"#,
        )
        .expect("popup CSS.registerProperty setup should evaluate");

    assert_eq!(
        initial,
        "rgb(0, 0, 0)|rgb(0, 0, 0)|rgb(7, 8, 9)|480px|540px|color: rgb(7, 8, 9); width: 25%; height: 50%;|true"
    );
    let popup_document = owner_document_handle_for_element_id(&vm, "popup-register-target");
    assert_ne!(popup_document, document);
    let document_cache_entries = computed_style_cache_entry_count_for_document(&vm, document);
    let popup_cache_entries = computed_style_cache_entry_count_for_document(&vm, popup_document);
    assert!(document_cache_entries > 0);
    assert!(popup_cache_entries > 0);
    let document_stylist_identity = vm.retained_stylist_identity_for_document_for_test(document);
    let popup_stylist_identity = vm.retained_stylist_identity_for_document_for_test(popup_document);
    let document_rebuilds = vm.retained_style_system_rebuild_count_for_document_for_test(document);
    let popup_rebuilds =
        vm.retained_style_system_rebuild_count_for_document_for_test(popup_document);
    let document_updates = vm.retained_style_system_update_count_for_document_for_test(document);
    let popup_updates = vm.retained_style_system_update_count_for_document_for_test(popup_document);
    assert!(!registered_custom_property_for_document(
        &vm,
        document,
        "--popup-token"
    ));
    assert!(!registered_custom_property_for_document(
        &vm,
        popup_document,
        "--popup-token"
    ));

    let registered = vm
        .eval(
            r#"
(() => {
  __styleRegisterPopup.CSS.registerProperty({
    name: '--popup-token',
    syntax: '<color>',
    initialValue: 'rgb(40, 50, 60)',
    inherits: false
  });
  return typeof __styleRegisterPopup.CSS.registerProperty;
})()
"#,
        )
        .expect("popup CSS.registerProperty should evaluate");

    assert_eq!(registered, "function");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, document),
        document_cache_entries
    );
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, popup_document),
        popup_cache_entries,
        "popup registration must defer invalidation until its next observation"
    );
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(document),
        document_stylist_identity
    );
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(popup_document),
        popup_stylist_identity
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        document_rebuilds
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(popup_document),
        popup_rebuilds
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(document),
        document_updates
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(popup_document),
        popup_updates,
        "registration must only mark the popup style world dirty"
    );
    assert!(!registered_custom_property_for_document(
        &vm,
        document,
        "--popup-token"
    ));
    assert!(registered_custom_property_for_document(
        &vm,
        popup_document,
        "--popup-token"
    ));

    let resolved = vm
        .eval(
            r#"
(() => {
  const active = document.getElementById('active-popup-register-target');
  const popupTarget = __styleRegisterPopup.document.getElementById('popup-register-target');
  return [
    getComputedStyle(active).color,
    __styleRegisterPopup.getComputedStyle(popupTarget).color,
    __styleRegisterPopup.getComputedStyle(popupTarget).getPropertyValue('color'),
    __styleRegisterPopup.getComputedStyle(popupTarget).getPropertyValue('--popup-token')
  ].join('|');
})()
"#,
        )
        .expect("computed styles should use each document registration");

    assert_eq!(
        resolved,
        "rgb(0, 0, 0)|rgb(40, 50, 60)|rgb(40, 50, 60)|rgb(40, 50, 60)"
    );
    assert!(computed_style_cache_entry_count_for_document(&vm, document) > 0);
    assert!(computed_style_cache_entry_count_for_document(&vm, popup_document) > 0);
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(document),
        document_stylist_identity
    );
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(popup_document),
        popup_stylist_identity
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        document_rebuilds
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(popup_document),
        popup_rebuilds
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(document),
        document_updates,
        "reading the clean opener document must not update its style world"
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(popup_document),
        popup_updates + 1,
        "the first popup observation must update its retained Stylist once"
    );
}

#[test]
fn popup_held_computed_style_wrapper_is_empty_after_about_blank_navigation() {
    let mut vm = new_storage_test_vm("https://popup-held-computed-navigation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const popup = open('about:blank');
  const target = popup.document.createElement('div');
  target.id = 'popup-held-computed-target';
  target.style.cssText = 'color: rgb(11, 22, 33);';
  popup.document.body.appendChild(target);
  const held = popup.getComputedStyle(target);
  const oldDocument = popup.document;
  const before = [held.color, held.length > 200].join(':');

  popup.location.href = 'about:blank?next';
  const replacement = popup.document.createElement('div');
  replacement.id = 'popup-held-computed-replacement';
  replacement.style.cssText = 'color: rgb(44, 55, 66);';
  popup.document.body.appendChild(replacement);

  return [
    before,
    popup.location.href,
    popup.document !== oldDocument,
    held.color,
    held.length,
    popup.getComputedStyle(replacement).color
  ].join('|');
})()
"#,
        )
        .expect("popup held computed style navigation should evaluate");

    assert_eq!(
        result,
        "rgb(11, 22, 33):true|about:blank?next|true||0|rgb(44, 55, 66)"
    );
    let retired_popup_document =
        owner_document_handle_for_element_id(&vm, "popup-held-computed-target");
    let current_popup_document =
        owner_document_handle_for_element_id(&vm, "popup-held-computed-replacement");
    assert_ne!(retired_popup_document, current_popup_document);
    assert!(!vm.document_style_world_is_active_for_test(retired_popup_document));
    assert!(vm.document_style_world_is_active_for_test(current_popup_document));
}

#[tokio::test]
async fn popup_held_computed_style_wrapper_is_empty_after_loaded_navigation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://popup-held-loaded-computed-navigation.test/",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  const popup = open('about:blank');
  globalThis.__popupLoadedComputedReady = false;
  globalThis.__popupLoadedComputedPopup = popup;
  const target = popup.document.createElement('div');
  target.style.cssText = 'color: rgb(11, 22, 33);';
  popup.document.body.appendChild(target);
  globalThis.__popupLoadedComputedHeld = popup.getComputedStyle(target);
  globalThis.__popupLoadedComputedOldDocument = popup.document;
  const html = `<!doctype html><body>
    <div id="replacement" style="color: rgb(44, 55, 66)">replacement</div>
    <script>opener.__popupLoadedComputedReady = true;<\/script>
  </body>`;
  globalThis.__popupLoadedComputedUrl =
    URL.createObjectURL(new Blob([html], { type: 'text/html' }));
  popup.location.href = __popupLoadedComputedUrl;
  return [
    __popupLoadedComputedHeld.color,
    __popupLoadedComputedHeld.length > 200,
    String(__popupLoadedComputedReady),
    popup.location.href === __popupLoadedComputedUrl
  ].join('|');
})()
"#,
        )
        .expect("popup loaded held computed style setup should evaluate");

    assert_eq!(setup, "rgb(11, 22, 33)|true|false|true");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__popupLoadedComputedReady)",
        "true",
        "loaded popup document should run",
    )
    .await;

    let result = vm
        .eval(
            r#"
(() => {
  const popup = __popupLoadedComputedPopup;
  const replacement = popup.document.getElementById('replacement');
  return [
    popup.document !== __popupLoadedComputedOldDocument,
    __popupLoadedComputedHeld.color,
    __popupLoadedComputedHeld.length,
    popup.getComputedStyle(replacement).color,
    popup.location.href === __popupLoadedComputedUrl
  ].join('|');
})()
"#,
        )
        .expect("popup loaded held computed style result should evaluate");

    assert_eq!(result, "true||0|rgb(44, 55, 66)|true");
}

#[test]
fn css_register_property_updates_var_substitution_computed_value() {
    let mut vm = new_storage_test_vm("https://css-register-property-var-cascade.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  const outer = document.createElement('div');
  outer.style.color = 'rgb(1, 1, 1)';
  const inner = document.createElement('div');
  inner.style.cssText = `
    --my-color: rgb(2, 2, 2);
    --my-color: url(not-a-color);
    color: var(--my-color);
  `;
  outer.appendChild(inner);
  root.appendChild(outer);

  const before = getComputedStyle(inner).color;
  CSS.registerProperty({
    name: '--my-color',
    syntax: '<color>',
    initialValue: 'rgb(3, 3, 3)',
    inherits: false
  });
  const after = getComputedStyle(inner).color;
  return `${before}|${after}`;
})()
"#,
        )
        .expect("registered custom property should update var substitution computed value");

    assert_eq!(result, "rgb(1, 1, 1)|rgb(3, 3, 3)");
}

#[test]
fn css_register_property_inline_var_mutation_preserves_existing_font_size_unset() {
    let mut vm =
        new_storage_test_vm("https://css-register-property-inline-var-preserve-unset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  CSS.registerProperty({
    name: '--x',
    syntax: '*',
    initialValue: '0px',
    inherits: false
  });
  const style = document.createElement('style');
  style.textContent = `
    :root, #target { --x: 2em; }
    #target { font-size: 11px; line-height: 13px; }
  `;
  head.appendChild(style);
  const target = document.createElement('div');
  target.id = 'target';
  body.appendChild(target);

  target.style.fontSize = 'unset';
  const before = getComputedStyle(target).fontSize;
  target.style.marginBottom = 'var(--x)';
  const after = getComputedStyle(target).fontSize;
  const margin = getComputedStyle(target).marginBottom;
  const cssText = target.style.cssText;
  target.remove();
  style.remove();
  return [cssText, before, after, margin].join('|');
})()
"#,
        )
        .expect("registered custom property inline var mutation should evaluate");

    assert_eq!(
        result,
        "font-size: unset; margin-bottom: var(--x);|16px|16px|32px"
    );
}

#[test]
fn registered_custom_ident_accepts_ident_function() {
    let mut vm = new_storage_test_vm("https://registered-custom-ident-function.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  CSS.registerProperty({
    name: '--ident',
    syntax: '<custom-ident>',
    inherits: true,
    initialValue: 'none'
  });
  const target = document.createElement('div');
  target.style.setProperty('--ident', 'ident("--myident" calc(42 * sign(1em - 1px)))');
  root.appendChild(target);
  const value = getComputedStyle(target).getPropertyValue('--ident');
  target.remove();
  return value;
})()
"#,
        )
        .expect("registered custom-ident ident function should evaluate");

    assert_eq!(result, "--myident42");
}

#[test]
fn registered_color_resolves_tree_counting_math() {
    let mut vm = new_storage_test_vm("https://registered-color-tree-counting.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    @property --color {
      inherits: false;
      initial-value: black;
      syntax: "<color>";
    }
    #target {
      --color: color(srgb 0 sibling-index() 0);
    }
  `;
  head.appendChild(style);
  const parent = document.createElement('div');
  const target = document.createElement('div');
  target.id = 'target';
  parent.appendChild(target);
  body.appendChild(parent);
  return getComputedStyle(target).getPropertyValue('--color');
})()
"#,
        )
        .expect("registered color tree-counting math should evaluate");

    assert_eq!(result, "color(srgb 0 1 0)");
}

#[test]
fn computed_style_resolves_border_width_from_border_shorthand() {
    let mut vm = new_storage_test_vm("https://computed-border-width.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('input');
  target.type = 'checkbox';
  target.style = 'border: 5px solid red';
  (document.body || document.documentElement || document).appendChild(target);
  const style = getComputedStyle(target);
  return [
    style.getPropertyValue('border-width'),
    style.getPropertyValue('border-top-width'),
    style.getPropertyValue('border-right-width'),
    style.getPropertyValue('border-bottom-width'),
    style.getPropertyValue('border-left-width')
  ].join('|');
})()
"#,
        )
        .expect("computed style should resolve border widths from border shorthand");

    assert_eq!(result, "5px|5px|5px|5px|5px");
}

#[test]
fn keyframe_rule_style_rejects_animation_properties() {
    let mut vm = new_storage_test_vm("https://keyframe-style-restrictions.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = '@keyframes foo { from { margin-top: 10px; animation-name: none; } }';
  (document.body || document.documentElement || document).appendChild(style);
  const declaration = document.styleSheets[0].cssRules[0].cssRules[0].style;
  const initial = `${declaration.length}|${declaration.marginTop}|${declaration.getPropertyValue('animation-name')}`;
  declaration.setProperty('animation-name', 'none');
  const afterSet = `${declaration.length}|${declaration.getPropertyValue('animation-name')}`;
  declaration.cssText = 'margin-bottom: 10px; animation-name: none;';
  const afterText = `${declaration.length}|${declaration.marginBottom}|${declaration.getPropertyValue('animation-name')}`;
  return `${initial}|${afterSet}|${afterText}`;
})()
"#,
        )
        .expect("keyframe style should reject animation properties");

    assert_eq!(result, "1|10px||1||1|10px|");
}

#[test]
fn computed_style_resolves_system_color_properties() {
    let mut vm = new_storage_test_vm("https://computed-system-colors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  target.id = 'target';
  const style = document.createElement('style');
  style.textContent = `
    #target {
      background-color: Menu;
      border: 1px solid Menu;
      box-shadow: 1px 1px MenuText, 2px 2px LinkText;
      caret-color: Menu;
      color: Menu;
      outline-color: Menu;
    }
  `;
  body.append(style, target);
  const computed = getComputedStyle(target);
  const properties = [
    'background-color',
    'border-top-color',
    'border-right-color',
    'border-bottom-color',
    'border-left-color',
    'box-shadow',
    'caret-color',
    'color',
    'outline-color'
  ];
  return JSON.stringify({
    allRgb: properties.every((property) => /^rgb/.test(computed.getPropertyValue(property))),
    boxShadow: computed.getPropertyValue('box-shadow')
  });
})()
"#,
        )
        .expect("computed style should resolve system colors");

    assert_eq!(
        result,
        r#"{"allRgb":true,"boxShadow":"rgb(0, 0, 0) 1px 1px 0px 0px, rgb(0, 0, 238) 2px 2px 0px 0px"}"#
    );
}

#[test]
fn meta_color_scheme_controls_used_system_colors_without_changing_computed_property() {
    let mut vm = new_parsed_test_vm(
        "https://meta-color-scheme.test/",
        r#"<!doctype html>
        <html><head>
          <meta id="scheme" http-equiv="content-language" name="color-scheme" content="dark">
        </head><body>
          <div id="light" style="color-scheme: only light; color: CanvasText"></div>
          <div id="dark" style="color-scheme: only dark; color: CanvasText"></div>
          <div id="normal" style="color-scheme: normal; color: CanvasText"></div>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const meta = document.getElementById('scheme');
  const light = getComputedStyle(document.getElementById('light')).color;
  const dark = getComputedStyle(document.getElementById('dark')).color;
  const normal = () => getComputedStyle(document.getElementById('normal')).color;
  const root = () => getComputedStyle(document.documentElement);
  const states = {
    explicitSchemesDiffer: light !== dark,
    initialRootUsesDark: root().color === dark,
    initialNormalUsesDark: normal() === dark,
    rootComputedSchemeStaysNormal: root().colorScheme === 'normal',
    metaDoesNotChangePreference: !matchMedia('(prefers-color-scheme: dark)').matches
  };

  meta.content = 'light';
  states.lightMutationUpdatesRoot = root().color === light;
  states.lightMutationUpdatesNormal = normal() === light;
  states.explicitDarkStaysDark =
    getComputedStyle(document.getElementById('dark')).color === dark;

  meta.content = ',,invalid';
  states.invalidContentUsesInitial = root().color === light;
  meta.removeAttribute('content');
  states.missingContentUsesInitial = root().color === light;

  meta.content = 'dark';
  const earlier = document.createElement('meta');
  earlier.name = 'color-scheme';
  earlier.content = 'light';
  document.head.insertBefore(earlier, meta);
  states.firstValidWins = root().color === light;
  earlier.remove();
  states.removalRestoresNextValid = root().color === dark;

  meta.remove();
  const host = document.body.appendChild(document.createElement('div'));
  const shadowMeta = document.createElement('meta');
  shadowMeta.name = 'color-scheme';
  shadowMeta.content = 'dark';
  host.attachShadow({ mode: 'open' }).appendChild(shadowMeta);
  states.shadowMetaIsIgnored = root().color === light;
  return JSON.stringify(states);
})()
"#,
        )
        .expect("meta color-scheme should control used system colors");

    assert_eq!(
        result,
        r#"{"explicitSchemesDiffer":true,"initialRootUsesDark":true,"initialNormalUsesDark":true,"rootComputedSchemeStaysNormal":true,"metaDoesNotChangePreference":true,"lightMutationUpdatesRoot":true,"lightMutationUpdatesNormal":true,"explicitDarkStaysDark":true,"invalidContentUsesInitial":true,"missingContentUsesInitial":true,"firstValidWins":true,"removalRestoresNextValid":true,"shadowMetaIsIgnored":true}"#
    );
}

#[test]
fn computed_style_resolves_env_color_fallbacks() {
    let mut vm = new_storage_test_vm("https://computed-env-color.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = 'div { background-color: rgb(0, 128, 0); }';
  body.appendChild(style);

  const values = [
    'env(test)',
    'ENV(test)',
    'env(test, blue)',
    'env(test, env(another, blue))',
    'env(test, {})',
    'env(env(test))'
  ];
  return values.map((value) => {
    const element = document.createElement('div');
    body.appendChild(element);
    element.style.backgroundColor = value;
    return getComputedStyle(element).getPropertyValue('background-color');
  }).join('|');
})()
"#,
        )
        .expect("computed env() color fallbacks should evaluate");

    assert_eq!(
        result,
        "rgba(0, 0, 0, 0)|rgba(0, 0, 0, 0)|rgb(0, 0, 255)|rgb(0, 0, 255)|rgba(0, 0, 0, 0)|rgb(0, 128, 0)"
    );
}

#[test]
fn computed_style_resolves_auto_min_size() {
    let mut vm = new_storage_test_vm("https://computed-auto-min-size.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  body.innerHTML = `
    <div id="plain"></div>
    <div id="ratio" style="aspect-ratio: 1/1"></div>
    <div style="display:flex"><div id="flexItem"></div></div>
    <div style="display:none"><div id="hiddenRatio" style="aspect-ratio: 1/1"></div></div>
  `;
  const ids = ['plain', 'ratio', 'flexItem', 'hiddenRatio'];
  return ids.map((id) => {
    const style = getComputedStyle(document.getElementById(id));
    return `${style.minWidth}/${style.minHeight}`;
  }).join('|');
})()
"#,
        )
        .expect("computed style should resolve auto min sizes");

    assert_eq!(result, "0px/0px|auto/auto|auto/auto|0px/0px");
}

#[test]
fn computed_style_enumerates_logical_longhands_only() {
    let mut vm = new_storage_test_vm("https://computed-style-logical.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(target);
  const properties = Array.from(getComputedStyle(target));
  return JSON.stringify({
    blockSize: properties.includes('block-size'),
    paddingBlock: properties.includes('padding-block'),
    safeAreaInsetTop: properties.includes('safe-area-inset-top'),
  });
})()
"#,
        )
        .expect("computed style logical property enumeration should evaluate");

    assert_eq!(
        result,
        r#"{"blockSize":true,"paddingBlock":false,"safeAreaInsetTop":false}"#
    );
}

#[test]
fn detached_child_document_style_sheet_survives_iframe_removal() {
    let mut vm = new_storage_test_vm("https://stylesheet-removed-frame.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  frame.contentDocument.body.innerHTML = '<style>div { color: red; }</style>';
  const sheet = frame.contentDocument.querySelector('style').sheet;
  const before = sheet && sheet.cssRules.length;
  frame.remove();
  sheet.insertRule('span { color: green; }', 0);
  return [before, sheet.cssRules.length].join(',');
})()
"#,
        )
        .expect("removed child document stylesheet should remain mutable");

    assert_eq!(result, "1,2");
}

#[test]
fn live_inline_font_shorthand_serializes_line_height_slash_spacing() {
    let mut vm = new_storage_test_vm("https://font-shorthand-css-text.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  target.setAttribute('style', 'font: 10px/1 Ahem;');
  const attribute = [target.style.cssText, target.style.font].join('|');
  target.style.font = 'italic 16px/2 "A B", serif';
  const setter = [target.style.cssText, target.style.font].join('|');
  return `${attribute}||${setter}`;
})()
"#,
        )
        .expect("font shorthand serialization should evaluate");

    assert_eq!(
        result,
        r#"font: 10px / 1 Ahem;|10px / 1 Ahem||font: italic 16px / 2 "A B", serif;|italic 16px / 2 "A B", serif"#
    );
}

#[test]
fn computed_color_normalizes_named_and_hex_colors() {
    let mut vm = new_storage_html_test_vm("https://computed-color-normalization.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const values = ['white', 'orange', 'rebeccapurple', 'transparent', '#0f8', '#112233'];
  return values.map((value) => {
    const element = document.createElement('div');
    element.style.color = value;
    document.body.appendChild(element);
    return getComputedStyle(element).color;
  }).join('|');
})()
"##,
        )
        .expect("computed color normalization should evaluate");

    assert_eq!(
        result,
        "rgb(255, 255, 255)|rgb(255, 165, 0)|rgb(102, 51, 153)|rgba(0, 0, 0, 0)|rgb(0, 255, 136)|rgb(17, 34, 51)"
    );
}

#[test]
fn computed_currentcolor_resolves_for_text_decoration_independent_of_webkit_text_fill() {
    let mut vm = new_storage_test_vm("https://computed-text-fill-currentcolor.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('p');
  target.style.cssText = 'text-decoration-color: currentColor; color: blue; -webkit-text-fill-color: red;';
  (document.body || document.documentElement || document).appendChild(target);
  const computed = getComputedStyle(target);
  return [
    computed.getPropertyValue('text-decoration-color'),
    computed.getPropertyValue('color'),
    computed.getPropertyValue('-webkit-text-fill-color')
  ].join('|');
})()
"#,
        )
        .expect("computed text decoration color should resolve currentColor");

    assert_eq!(result, "rgb(0, 0, 255)|rgb(0, 0, 255)|rgb(255, 0, 0)");
}

#[test]
fn computed_color_inherits_through_shadow_host_and_hidden_subtree() {
    let mut vm = new_storage_test_vm("https://computed-color-inheritance.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = '#container { color: red }';
  (document.head || document.documentElement || document).appendChild(style);
  const container = document.createElement('div');
  container.id = 'container';
  const host = document.createElement('div');
  const hidden = document.createElement('div');
  hidden.style.display = 'none';
  const hiddenChild = document.createElement('span');
  hidden.appendChild(hiddenChild);
  container.append(host, hidden);
  (document.body || document.documentElement || document).appendChild(container);
  const root = host.attachShadow({ mode: 'open' });
  root.innerHTML = '<div id="target"></div>';
  return [
    getComputedStyle(root.getElementById('target')).color,
    getComputedStyle(hiddenChild).color
  ].join('|');
})()
"#,
        )
        .expect("computed color should inherit through shadow and hidden subtrees");

    assert_eq!(result, "rgb(255, 0, 0)|rgb(255, 0, 0)");
}

#[test]
fn target_pseudo_updates_computed_style_after_fragment_changes() {
    let mut vm = new_parsed_test_vm(
        "https://target-style.test/#old",
        r#"
        <html>
          <head>
            <style>
              .probe { color: rgb(0, 0, 0); }
              #old:target { color: rgb(255, 0, 0); }
              #new:target { color: rgb(0, 128, 0); }
            </style>
          </head>
          <body>
            <div id="old" class="probe"></div>
            <div id="new" class="probe"></div>
          </body>
        </html>
        "#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const oldTarget = document.getElementById('old');
  const newTarget = document.getElementById('new');
  const read = () => [
    oldTarget.matches(':target'),
    newTarget.matches(':target'),
    getComputedStyle(oldTarget).color,
    getComputedStyle(newTarget).color
  ].join('/');

  const initial = read();
  history.replaceState(null, '', '#new');
  const replaced = read();
  history.replaceState(null, '', '#missing');
  const missing = read();

  return [initial, replaced, missing].join('|');
})()
"#,
        )
        .expect("target pseudo computed style should update after fragment changes");

    assert_eq!(
        result,
        "true/false/rgb(255, 0, 0)/rgb(0, 0, 0)|false/true/rgb(0, 0, 0)/rgb(0, 128, 0)|false/false/rgb(0, 0, 0)/rgb(0, 0, 0)"
    );
}

#[test]
fn target_selector_fragment_change_invalidates_held_computed_style() {
    let mut vm = new_parsed_test_vm(
        "https://target-held-style.test/#old",
        r#"
        <html>
          <head>
            <style>
              .probe { color: rgb(0, 0, 0); }
              #old:target { color: rgb(255, 0, 0); }
              #new:target { color: rgb(0, 128, 0); }
            </style>
          </head>
          <body>
            <div id="old" class="probe"></div>
            <div id="new" class="probe"></div>
          </body>
        </html>
        "#,
    );
    let document = vm.document_handle_for_test();

    let setup = vm
        .eval(
            r#"
(() => {
  const oldTarget = document.getElementById('old');
  const newTarget = document.getElementById('new');
  globalThis.__targetOldStyle = getComputedStyle(oldTarget);
  globalThis.__targetNewStyle = getComputedStyle(newTarget);
  return [globalThis.__targetOldStyle.color, globalThis.__targetNewStyle.color].join('|');
})()
"#,
        )
        .expect("target selector held style setup should evaluate");

    assert_eq!(setup, "rgb(255, 0, 0)|rgb(0, 0, 0)");
    let generation_before_fragment =
        vm.computed_style_cache_generation_for_document_for_test(document);

    let replaced = vm
        .eval(
            r#"
(() => {
  history.replaceState(null, '', '#new');
  const result = [globalThis.__targetOldStyle.color, globalThis.__targetNewStyle.color].join('|');
  delete globalThis.__targetOldStyle;
  delete globalThis.__targetNewStyle;
  return result;
})()
"#,
        )
        .expect("target selector held style mutation should evaluate");

    assert_eq!(replaced, "rgb(0, 0, 0)|rgb(0, 128, 0)");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_before_fragment,
        "targeted :target invalidation should not bump the retained style generation"
    );
}

#[test]
fn child_frame_target_selector_invalidation_uses_child_document_world() {
    let mut vm = new_storage_test_vm("https://child-target-style-cache.test/");

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const activeStyle = document.createElement('style');
  activeStyle.textContent = '#active-target-cache { color: rgb(1, 2, 3); }';
  head.appendChild(activeStyle);
  const active = document.createElement('div');
  active.id = 'active-target-cache';
  body.appendChild(active);
  globalThis.__childTargetActiveStyle = getComputedStyle(active);

  const frame = document.createElement('iframe');
  frame.id = 'target-child-frame';
  body.appendChild(frame);
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write(`
    <style>
      .probe { color: rgb(4, 5, 6); }
      #old:target { color: rgb(7, 8, 9); }
      #new:target { color: rgb(10, 11, 12); }
    </style>
    <body>
      <div id="old" class="probe"></div>
      <div id="new" class="probe"></div>
    </body>
  `);
  childDocument.close();
  childWindow.history.replaceState(null, '', '#old');
  globalThis.__childTargetFrame = frame;
  globalThis.__childTargetOldStyle =
    childWindow.getComputedStyle(childDocument.getElementById('old'));
  globalThis.__childTargetNewStyle =
    childWindow.getComputedStyle(childDocument.getElementById('new'));

  return [
    globalThis.__childTargetActiveStyle.color,
    globalThis.__childTargetOldStyle.color,
    globalThis.__childTargetNewStyle.color
  ].join('|');
})()
"#,
        )
        .expect("child frame target style setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(7, 8, 9)|rgb(4, 5, 6)");
    let active_document = vm.document_runtime.dom_host().document_handle();
    let child_document = child_document_handle_for_frame_id(&vm, "target-child-frame");
    let active_cache_before = computed_style_cache_entry_count_for_document(&vm, active_document);
    assert!(active_cache_before > 0);
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, child_document),
        2
    );

    let replaced = vm
        .eval(
            r#"
(() => {
  const childWindow = globalThis.__childTargetFrame.contentWindow;
  childWindow.history.replaceState(null, '', '#new');
  const result = [
    globalThis.__childTargetOldStyle.color,
    globalThis.__childTargetNewStyle.color
  ].join('|');
  delete globalThis.__childTargetFrame;
  delete globalThis.__childTargetOldStyle;
  delete globalThis.__childTargetNewStyle;
  delete globalThis.__childTargetActiveStyle;
  return result;
})()
"#,
        )
        .expect("child frame target style mutation should evaluate");

    assert_eq!(replaced, "rgb(4, 5, 6)|rgb(10, 11, 12)");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, active_document),
        active_cache_before,
        "child target invalidation should not clear active document cache"
    );
    assert!(
        computed_style_cache_entry_count_for_document(&vm, child_document) > 0,
        "child target invalidation should keep style work in the child document world"
    );
}
