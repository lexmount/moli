use super::*;

#[test]
fn css_highlights_declared_property_preserves_descriptor() {
    let mut vm = new_storage_test_vm("https://css-highlights-declared.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptor = Object.getOwnPropertyDescriptor(CSS, "highlights");
  const before = CSS.highlights;
  const reflectSet = Reflect.set(CSS, "highlights", "poison");
  CSS.highlights = "poison";
  return [
    !!descriptor,
    descriptor && descriptor.enumerable,
    descriptor && descriptor.writable,
    descriptor && descriptor.configurable,
    descriptor && typeof descriptor.value,
    descriptor && descriptor.value === before,
    CSS.highlights === before,
    reflectSet,
    Object.keys(CSS).join(","),
    typeof Highlight,
    typeof CSS.highlights.set,
    CSS.highlights.set("declared", new Highlight()) === CSS.highlights,
    CSS.highlights.has("declared")
  ].join("|");
})()
"#,
        )
        .expect("CSS.highlights declared descriptor should evaluate");

    assert_eq!(
        result,
        "true|false|false|true|object|true|true|false|escape,registerProperty,supports|function|function|true|true"
    );
}
#[test]
fn font_face_declared_slots_ignore_prototype_spoofing() {
    let mut vm = new_storage_test_vm("https://font-face-declared-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const stringify = value => value === undefined ? 'undefined' : String(value);
  const descriptorShape = (prototype, receiver, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      typeof descriptor.get,
      descriptor.get && descriptor.get.name,
      descriptor.get && descriptor.get.length,
      typeof descriptor.set,
      descriptor.set && descriptor.set.name,
      descriptor.set && descriptor.set.length,
      descriptor.enumerable,
      descriptor.configurable,
      Object.prototype.hasOwnProperty.call(receiver, name)
    ].map(stringify).join(':');
  };
  const face = new FontFace('Demo', 'url(demo.woff)', {
    style: 'italic',
    weight: '700',
    stretch: 'condensed',
    variant: 'small-caps',
    featureSettings: '"kern"',
    display: 'swap'
  });
  const ownSlots = Object.getOwnPropertyNames(face)
    .filter(name => name.startsWith('__moliFontFace'))
    .sort();
  face.family = 'Changed';
  FontFace.prototype.__moliFontFaceFamily = 'PrototypeFamily';
  FontFace.prototype.__moliFontFaceSource = 'PrototypeSource';
  FontFace.prototype.__moliFontFaceStyle = 'normal';
  FontFace.prototype.__moliFontFaceStatus = 'error';
  FontFace.prototype.__moliFontFaceLoaded = Promise.resolve('bad');
  face.__moliFontFaceFamily = 'OwnFamily';
  face.__moliFontFaceSource = 'OwnSource';
  face.__moliFontFaceStyle = 'normal';
  face.__moliFontFaceWeight = '100';
  face.__moliFontFaceStretch = 'expanded';
  face.__moliFontFaceVariant = 'normal';
  face.__moliFontFaceFeatureSettings = 'normal';
  face.__moliFontFaceDisplay = 'auto';
  face.__moliFontFaceStatus = 'error';
  face.__moliFontFaceLoaded = Promise.resolve('ownBad');
  const fake = Object.create(FontFace.prototype);
  globalThis.fakeLoadedResult = 'not called';
  return JSON.stringify({
    values: [
      face.family,
      face.source,
      face.style,
      face.weight,
      face.stretch,
      face.variant,
      face.featureSettings,
      face.display,
      face.status,
      typeof face.loaded.then
    ].join('|'),
    fake: ['family', 'source', 'style', 'status', 'loaded'].map(name => {
      try {
        const value = fake[name];
        if (name === 'loaded' && value instanceof Promise) {
          value.then(
            () => fakeLoadedResult = 'resolved',
            error => fakeLoadedResult = error instanceof TypeError ? 'rejected:TypeError' : error.name
          );
          return 'Promise';
        }
        return String(value);
      } catch (error) { return error.name; }
    }).join('|'),
    descriptors: [
      'family',
      'style',
      'weight',
      'stretch',
      'variant',
      'featureSettings',
      'display',
      'source',
      'status',
      'loaded'
    ].map(name => descriptorShape(FontFace.prototype, face, name)),
    ownSlots
  });
})()
"#,
        )
        .expect("FontFace declared slots should ignore prototype spoofing");

    assert_eq!(
        result,
        r#"{"values":"Changed|url(demo.woff)|italic|700|condensed|small-caps|\"kern\"|swap|loaded|function","fake":"TypeError|TypeError|TypeError|TypeError|Promise","descriptors":["family:function:get family:0:function:set family:1:true:true:false","style:function:get style:0:function:set style:1:true:true:false","weight:function:get weight:0:function:set weight:1:true:true:false","stretch:function:get stretch:0:function:set stretch:1:true:true:false","variant:function:get variant:0:function:set variant:1:true:true:false","featureSettings:function:get featureSettings:0:function:set featureSettings:1:true:true:false","display:function:get display:0:function:set display:1:true:true:false","source:function:get source:0:undefined:undefined:undefined:true:true:false","status:function:get status:0:undefined:undefined:undefined:true:true:false","loaded:function:get loaded:0:undefined:undefined:undefined:true:true:false"],"ownSlots":[]}"#
    );
    assert_eq!(vm.eval("fakeLoadedResult").unwrap(), "rejected:TypeError");
}
#[test]
fn font_face_variation_settings_use_stylo_descriptor_serialization() {
    let mut vm = new_storage_test_vm("https://font-face-variation-settings.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const face = new FontFace('Variable', 'url(variable.woff2)', {
    variationSettings: "'wght' 850"
  });
  const initial = face.variationSettings;
  face.variationSettings = "'wdth' 120.5";
  const updated = face.variationSettings;
  let setterError = '';
  try {
    face.variationSettings = 'not-a-valid-setting';
  } catch (error) {
    setterError = error.name;
  }
  const invalid = new FontFace('Invalid', 'url(invalid.woff2)', {
    variationSettings: 'not-a-valid-setting'
  });
  invalid.loaded.catch(() => {});
  const descriptor = Object.getOwnPropertyDescriptor(
    FontFace.prototype,
    'variationSettings'
  );
  return JSON.stringify({
    values: [initial, updated, face.variationSettings],
    setterError,
    constructorStatus: invalid.status,
    descriptor: [
      typeof descriptor.get,
      descriptor.get.length,
      typeof descriptor.set,
      descriptor.set.length,
      descriptor.enumerable,
      descriptor.configurable,
      Object.prototype.hasOwnProperty.call(face, 'variationSettings')
    ]
  });
})()
"#,
        )
        .expect("FontFace variationSettings serialization should evaluate");

    assert_eq!(
        result,
        r#"{"values":["\"wght\" 850","\"wdth\" 120.5","\"wdth\" 120.5"],"setterError":"SyntaxError","constructorStatus":"error","descriptor":["function",0,"function",1,true,true,false]}"#
    );
}
#[test]
fn font_face_set_load_event_copies_and_freezes_fontfaces() {
    let mut vm = new_storage_test_vm("https://font-face-set-load-event.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const face = new FontFace('Demo', 'url(demo.woff)');
  const source = [face];
  const empty = new FontFaceSetLoadEvent('loading');
  const configured = new FontFaceSetLoadEvent('loadingdone', {
    bubbles: true,
    cancelable: true,
    composed: true,
    fontfaces: source
  });
  source.length = 0;

  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };

  const fonts = document.implementation.createHTMLDocument('').fonts;
  fonts.add(face);
  let loadingShape = '';
  let doneShape = '';
  fonts.addEventListener('loading', event => {
    loadingShape = [
      event instanceof FontFaceSetLoadEvent,
      event.target === fonts,
      event.currentTarget === fonts,
      event.fontfaces.length
    ].join(':');
  });
  fonts.addEventListener('loadingdone', event => {
    doneShape = [
      event instanceof FontFaceSetLoadEvent,
      event.target === fonts,
      event.currentTarget === fonts,
      event.fontfaces.length,
      event.fontfaces[0] === face,
      Object.isFrozen(event.fontfaces)
    ].join(':');
  });
  fonts.load('10px Demo');

  const descriptor = Object.getOwnPropertyDescriptor(
    FontFaceSetLoadEvent.prototype,
    'fontfaces'
  );
  return JSON.stringify({
    surface: [
      typeof FontFaceSetLoadEvent,
      FontFaceSetLoadEvent.length,
      Object.getPrototypeOf(FontFaceSetLoadEvent.prototype) === Event.prototype,
      configured instanceof Event,
      Object.prototype.toString.call(configured)
    ],
    empty: [
      Array.isArray(empty.fontfaces),
      empty.fontfaces.length,
      empty.fontfaces === empty.fontfaces,
      Object.isFrozen(empty.fontfaces)
    ],
    configured: [
      configured.type,
      configured.bubbles,
      configured.cancelable,
      configured.composed,
      configured.fontfaces.length,
      configured.fontfaces[0] === face,
      configured.fontfaces !== source,
      Object.isFrozen(configured.fontfaces)
    ],
    descriptor: [
      typeof descriptor.get,
      descriptor.set,
      descriptor.enumerable,
      descriptor.configurable,
      Object.prototype.hasOwnProperty.call(configured, 'fontfaces')
    ].map(value => value === undefined ? 'undefined' : String(value)),
    errors: [
      throwsTypeError(() => FontFaceSetLoadEvent('loading')),
      throwsTypeError(() => new FontFaceSetLoadEvent()),
      throwsTypeError(() => new FontFaceSetLoadEvent('loading', { fontfaces: [{}] })),
      throwsTypeError(() => new FontFaceSetLoadEvent('loading', { fontfaces: null }))
    ],
    dispatched: [loadingShape, doneShape]
  });
})()
"#,
        )
        .expect("FontFaceSetLoadEvent FrozenArray semantics should evaluate");

    assert_eq!(
        result,
        r#"{"surface":["function",1,true,true,"[object FontFaceSetLoadEvent]"],"empty":[true,0,true,true],"configured":["loadingdone",true,true,true,1,true,true,true],"descriptor":["function","undefined","true","true","false"],"errors":[true,true,true,true],"dispatched":["true:true:true:0","true:true:true:1:true:true"]}"#
    );
}
#[test]
fn font_face_set_listeners_use_event_listener_callback_interface_semantics() {
    let mut vm = new_storage_test_vm("https://font-face-set-callback-interface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const fonts = document.implementation.createHTMLDocument('').fonts;
  const calls = [];

  let operationGets = 0;
  const objectListener = {};
  Object.defineProperty(objectListener, "handleEvent", {
    configurable: true,
    get() {
      operationGets++;
      return function(event) {
        calls.push(
          `object:${this === objectListener}:${event.currentTarget === fonts}:${window.event === event}`
        );
      };
    }
  });
  fonts.addEventListener("probe", objectListener);
  // A duplicate registration must not replace the original options.
  fonts.addEventListener("probe", objectListener, { once: true });

  let callableOperationGets = 0;
  function callable(event) {
    "use strict";
    calls.push(`callable:${this === fonts}:${event.currentTarget === fonts}`);
  }
  Object.defineProperty(callable, "handleEvent", {
    get() {
      callableOperationGets++;
      throw new Error("the callable branch must not resolve handleEvent");
    }
  });
  fonts.addEventListener("probe", callable);

  const removedBeforeVisit = () => calls.push("removed");
  const late = () => calls.push("late");
  fonts.addEventListener("probe", () => {
    calls.push("mutator");
    fonts.removeEventListener("probe", removedBeforeVisit);
    fonts.addEventListener("probe", late);
  });
  fonts.addEventListener("probe", removedBeforeVisit);

  let onceCalls = 0;
  const once = () => {
    onceCalls++;
    // A once listener is removed before callback entry, so this is a fresh
    // registration for the next dispatch.
    fonts.addEventListener("probe", once, { once: true });
  };
  fonts.addEventListener("probe", once, { once: true });

  const controller = new AbortController();
  fonts.addEventListener(
    "probe",
    () => calls.push("aborted"),
    { signal: controller.signal }
  );
  controller.abort();

  const supplied = new Event("probe");
  let suppliedSeen = false;
  fonts.addEventListener("probe", event => {
    suppliedSeen ||= event === supplied;
  });
  fonts.dispatchEvent(supplied);
  fonts.dispatchEvent(new Event("probe"));

  fonts.removeEventListener("probe", objectListener);
  fonts.removeEventListener("probe", callable);
  fonts.dispatchEvent(new Event("probe"));

  return JSON.stringify({
    calls,
    operationGets,
    callableOperationGets,
    onceCalls,
    suppliedSeen
  });
})()
"#,
        )
        .expect("FontFaceSet callback-interface semantics should evaluate");

    assert_eq!(
        result,
        r#"{"calls":["object:true:true:true","callable:true:true","mutator","object:true:true:true","callable:true:true","mutator","late","mutator","late"],"operationGets":2,"callableOperationGets":0,"onceCalls":3,"suppliedSeen":true}"#
    );
}
#[test]
fn font_face_set_listener_uses_callback_realm_and_exact_window_lifetime() {
    let mut vm = new_parsed_test_vm(
        "https://font-face-set-callback-realm.test/",
        "<!doctype html><html><body></body></html>",
    );

    vm.eval(
        r#"
(() => {
  const iframe = document.createElement("iframe");
  iframe.srcdoc = "<!doctype html><html><body></body></html>";
  document.body.appendChild(iframe);
  globalThis.__fontFaceSetCallbackRealmFrame = iframe;
})()
"#,
    )
    .expect("cross-Realm FontFaceSet listener setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const iframe = __fontFaceSetCallbackRealmFrame;
  const other = iframe.contentWindow;
  const fonts = document.implementation.createHTMLDocument('').fonts;
  globalThis.__fontFaceSetCallbackRealmTarget = fonts;
  globalThis.__fontFaceSetCallbackExpectedRealm = other;
  globalThis.__fontFaceSetCallbackRealmFacts = [];

  const callback = other.Function(
    "event",
    `"use strict";
     parent.__fontFaceSetCallbackRealmFacts.push([
       this === parent.__fontFaceSetCallbackRealmTarget,
       globalThis === parent.__fontFaceSetCallbackExpectedRealm,
       window.event === event,
       event.currentTarget === parent.__fontFaceSetCallbackRealmTarget
     ]);`
  );
  fonts.addEventListener("probe", callback);
  fonts.dispatchEvent(new Event("probe"));

  iframe.remove();
  fonts.dispatchEvent(new Event("probe"));

  return JSON.stringify({
    facts: __fontFaceSetCallbackRealmFacts,
    childDetached: iframe.contentWindow === null
  });
})()
"#,
        )
        .expect("cross-Realm FontFaceSet listener invocation should evaluate");

    assert_eq!(
        result,
        r#"{"facts":[[true,true,true,true]],"childDetached":true}"#
    );
}
#[test]
fn font_face_load_updates_owner_sets_once_and_unlinks_removed_faces() {
    let mut vm = new_storage_test_vm("https://font-face-owner-sets.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const face = new FontFace('Demo', 'url(demo.woff)');
  const documentFonts = document.fonts;
  const secondary = document.implementation.createHTMLDocument('').fonts;
  documentFonts.add(face);
  secondary.add(face);

  const events = [];
  const observe = (set, label) => {
    for (const type of ['loading', 'loadingdone']) {
      set.addEventListener(type, event => {
        events.push([
          label,
          event.type,
          event instanceof FontFaceSetLoadEvent,
          event.target === set,
          event.currentTarget === set,
          event.fontfaces.length,
          event.fontfaces.length === 0 || event.fontfaces[0] === face,
          Object.isFrozen(event.fontfaces)
        ].join(':'));
      });
    }
  };
  observe(documentFonts, 'document');
  observe(secondary, 'secondary');

  const documentReadyBefore = documentFonts.ready;
  const secondaryReadyBefore = secondary.ready;
  const firstLoad = face.load();
  const documentReadyAfter = documentFonts.ready;
  const secondaryReadyAfter = secondary.ready;
  const eventCountAfterFirstLoad = events.length;
  const secondLoad = face.load();

  const removed = new FontFace('Removed', 'url(removed.woff)');
  documentFonts.add(removed);
  const removedDeleted = documentFonts.delete(removed);
  const documentReadyBeforeRemovedLoad = documentFonts.ready;
  const eventCountBeforeRemovedLoad = events.length;
  removed.load();

  const clearedSet = document.implementation.createHTMLDocument('').fonts;
  const cleared = new FontFace('Cleared', 'url(cleared.woff)');
  clearedSet.add(cleared);
  const clearedReadyBeforeLoad = clearedSet.ready;
  clearedSet.clear();
  const eventCountBeforeClearedLoad = events.length;
  cleared.load();

  return JSON.stringify({
    loads: [firstLoad === face.loaded, secondLoad === face.loaded],
    ready: [
      documentReadyBefore !== documentReadyAfter,
      secondaryReadyBefore !== secondaryReadyAfter,
      documentReadyAfter === documentFonts.ready,
      secondaryReadyAfter === secondary.ready
    ],
    status: [documentFonts.status, secondary.status],
    events,
    repeated: events.length === eventCountAfterFirstLoad,
    removed: [
      removedDeleted,
      events.length === eventCountBeforeRemovedLoad,
      documentFonts.ready === documentReadyBeforeRemovedLoad
    ],
    cleared: [
      events.length === eventCountBeforeClearedLoad,
      clearedSet.ready === clearedReadyBeforeLoad,
      clearedSet.status === 'loaded'
    ]
  });
})()
"#,
        )
        .expect("FontFace loads should update each current owner set once");

    assert_eq!(
        result,
        r#"{"loads":[true,true],"ready":[true,true,true,true],"status":["loaded","loaded"],"events":["document:loading:true:true:true:0:true:true","document:loadingdone:true:true:true:1:true:true","secondary:loading:true:true:true:0:true:true","secondary:loadingdone:true:true:true:1:true:true"],"repeated":true,"removed":[true,true,true],"cleared":[true,true,true]}"#
    );
}
#[test]
fn stylesheet_font_face_load_updates_document_font_set() {
    let mut vm = new_storage_test_vm("https://stylesheet-font-face-owner.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const observed = [];
  let face;
  for (const type of ['loading', 'loadingdone']) {
    document.fonts.addEventListener(type, event => {
      observed.push([
        event.type,
        event.fontfaces.length,
        event.fontfaces.length === 0 || event.fontfaces[0] === face,
        Object.isFrozen(event.fontfaces)
      ].join(':'));
    });
  }
  const style = document.createElement('style');
  style.textContent = '@font-face { font-family: "StylesheetOwner"; src: url(owner.woff); }';
  (document.head || document.documentElement || document).appendChild(style);
  face = [...document.fonts].find(face => face.family === 'StylesheetOwner');
  const readyBefore = document.fonts.ready;
  const loaded = face.load();
  return JSON.stringify({
    face: face instanceof FontFace,
    loaded: loaded === face.loaded,
    ready: readyBefore !== document.fonts.ready,
    status: document.fonts.status,
    observed
  });
})()
"#,
        )
        .expect("stylesheet FontFace loads should update document.fonts");

    assert_eq!(
        result,
        r#"{"face":true,"loaded":true,"ready":true,"status":"loaded","observed":["loading:0:true:true","loadingdone:1:true:true"]}"#
    );
}
#[test]
fn document_fonts_tracks_live_document_adopted_stylesheets_without_duplicate_occurrences() {
    let mut vm = new_storage_test_vm("https://adopted-stylesheet-font-face.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@font-face { font-family: DocAdopted; src: local("Arial"); }');
  document.adoptedStyleSheets = [sheet, sheet];
  const fonts = document.fonts;
  const first = [...fonts].find(face => face.family === 'DocAdopted');
  first.marker = 42;
  const duplicateSize = fonts.size;

  sheet.disabled = true;
  const disabledSize = fonts.size;
  sheet.disabled = false;
  const afterDisabled = [...fonts].find(face => face.family === 'DocAdopted');

  sheet.media.mediaText = 'print';
  const printSize = fonts.size;
  sheet.media.mediaText = 'screen';
  const afterMedia = [...fonts].find(face => face.family === 'DocAdopted');

  const host = body.appendChild(document.createElement('div'));
  const shadow = host.attachShadow({ mode: 'open' });
  const shadowSheet = new CSSStyleSheet();
  shadowSheet.replaceSync('@font-face { font-family: ShadowAdopted; src: local("Arial"); }');
  shadow.adoptedStyleSheets = [shadowSheet];
  const shadowVisible = [...fonts].some(face => face.family === 'ShadowAdopted');

  sheet.insertRule('@font-face { font-family: SecondAdopted; src: local("Arial"); }');
  const afterInsert = [...fonts].map(face => face.family).sort().join(',');
  const afterInsertFirst = [...fonts].find(face => face.family === 'DocAdopted');

  document.adoptedStyleSheets = [];
  const removedSize = fonts.size;
  document.adoptedStyleSheets = [sheet];
  const readded = [...fonts].find(face => face.family === 'DocAdopted');
  return JSON.stringify({
    duplicateSize,
    disabledSize,
    disabledIdentity: first === afterDisabled && afterDisabled.marker === 42,
    printSize,
    mediaIdentity: first === afterMedia && afterMedia.marker === 42,
    shadowVisible,
    afterInsert,
    insertIdentity: first === afterInsertFirst && afterInsertFirst.marker === 42,
    removedSize,
    readdIdentity: first === readded && readded.marker === 42
  });
})()
"#,
        )
        .expect("document adopted stylesheet FontFaceSet projection should evaluate");

    assert_eq!(
        result,
        r#"{"duplicateSize":1,"disabledSize":0,"disabledIdentity":true,"printSize":0,"mediaIdentity":true,"shadowVisible":false,"afterInsert":"DocAdopted,SecondAdopted","insertIdentity":true,"removedSize":0,"readdIdentity":true}"#
    );
}
#[test]
fn document_fonts_tracks_effective_rules_and_native_rule_identity() {
    let mut vm = new_storage_test_vm("https://adopted-stylesheet-effective-font-face.test/");

    let initial = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  document.body || html.appendChild(document.createElement('body'));
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    @media print { @font-face { font-family: PrintOnly; src: local("Arial"); } }
    @media screen { @font-face { font-family: ScreenOnly; src: local("Arial"); } }
    @supports (display: block) { @font-face { font-family: Supported; src: local("Arial"); } }
    @supports (unknown-prop: impossible) { @font-face { font-family: Unsupported; src: local("Arial"); } }
  `);
  document.adoptedStyleSheets = [sheet];
  globalThis.__effectiveFontSheet = sheet;
  globalThis.__effectiveFonts = document.fonts;
  const screen = [...document.fonts].find(face => face.family === 'ScreenOnly');
  const supported = [...document.fonts].find(face => face.family === 'Supported');
  screen.marker = 'screen';
  supported.marker = 'supported';
  globalThis.__screenFont = screen;
  globalThis.__supportedFont = supported;
  return [...document.fonts].map(face => face.family).sort().join(',');
})()
"#,
        )
        .expect("effective adopted font-face setup should evaluate");

    assert_eq!(initial, "ScreenOnly,Supported");

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        media: Some("print".to_owned()),
        ..Default::default()
    });
    let print = vm
        .eval(
            r#"
JSON.stringify({
  names: [...globalThis.__effectiveFonts].map(face => face.family).sort(),
  supportedIdentity: [...globalThis.__effectiveFonts].find(face => face.family === 'Supported') === globalThis.__supportedFont,
  supportedMarker: globalThis.__supportedFont.marker
})
"#,
        )
        .expect("print emulation should refresh the retained FontFaceSet");
    assert_eq!(
        print,
        r#"{"names":["PrintOnly","Supported"],"supportedIdentity":true,"supportedMarker":"supported"}"#
    );

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides::default());
    let identity = vm
        .eval(
            r#"
(() => {
  const screen = [...globalThis.__effectiveFonts].find(face => face.family === 'ScreenOnly');
  const beforeDescriptor = screen === globalThis.__screenFont && screen.marker === 'screen';
  globalThis.__effectiveFontSheet.cssRules[1].cssRules[0].style.fontWeight = '700';
  const afterDescriptor = [...globalThis.__effectiveFonts].find(face => face.family === 'ScreenOnly');
  const descriptorChangedIdentity = afterDescriptor !== screen && afterDescriptor.marker === undefined;
  afterDescriptor.marker = 'after-descriptor';

  globalThis.__effectiveFontSheet.insertRule('body { color: red; }');
  const afterUnrelatedInsert = [...globalThis.__effectiveFonts].find(face => face.family === 'ScreenOnly');
  const unrelatedInsertIdentity = afterUnrelatedInsert === afterDescriptor && afterUnrelatedInsert.marker === 'after-descriptor';

  globalThis.__effectiveFontSheet.replaceSync('@font-face { font-family: ScreenOnly; src: local("Arial"); font-weight: 700; }');
  const afterReplace = [...globalThis.__effectiveFonts].find(face => face.family === 'ScreenOnly');
  const replaceChangedIdentity = afterReplace !== afterUnrelatedInsert && afterReplace.marker === undefined;
  afterReplace.marker = 'after-replace';
  globalThis.__effectiveFontSheet.deleteRule(0);
  globalThis.__effectiveFontSheet.insertRule('@font-face { font-family: ScreenOnly; src: local("Arial"); font-weight: 700; }', 0);
  const afterReinsert = [...globalThis.__effectiveFonts].find(face => face.family === 'ScreenOnly');
  afterReinsert.marker = 'after-reinsert';

  document.adoptedStyleSheets = [];
  globalThis.__effectiveFontSheet.deleteRule(0);
  globalThis.__effectiveFontSheet.insertRule('@font-face { font-family: ScreenOnly; src: local("Arial"); font-weight: 700; }', 0);
  document.adoptedStyleSheets = [globalThis.__effectiveFontSheet];
  const afterUnadoptedReinsert = [...globalThis.__effectiveFonts].find(face => face.family === 'ScreenOnly');
  return JSON.stringify({
    names: [...globalThis.__effectiveFonts].map(face => face.family).sort(),
    beforeDescriptor,
    descriptorChangedIdentity,
    unrelatedInsertIdentity,
    replaceChangedIdentity,
    reinsertChangedIdentity: afterReinsert !== afterReplace,
    unadoptedReinsertChangedIdentity:
      afterUnadoptedReinsert !== afterReinsert && afterUnadoptedReinsert.marker === undefined
  });
})()
"#,
        )
        .expect("stylesheet FontFace wrappers should follow native rule identity");

    assert_eq!(
        identity,
        r#"{"names":["ScreenOnly"],"beforeDescriptor":true,"descriptorChangedIdentity":true,"unrelatedInsertIdentity":true,"replaceChangedIdentity":true,"reinsertChangedIdentity":true,"unadoptedReinsertChangedIdentity":true}"#
    );
}
#[test]
fn font_face_binary_source_union_rejects_invalid_font_data() {
    let mut vm = new_storage_test_vm("https://font-face-binary-source.test/");

    vm.eval(
        r#"
(() => {
  const face = new FontFace('InvalidBinary', new ArrayBuffer(8));
  const fonts = document.implementation.createHTMLDocument('').fonts;
  fonts.add(face);
  const events = [];
  for (const type of ['loading', 'loadingdone', 'loadingerror']) {
    fonts.addEventListener(type, event => {
      events.push([
        event.type,
        event.fontfaces.length,
        event.fontfaces.length === 0 || event.fontfaces[0] === face
      ].join(':'));
    });
  }
  const readyBefore = fonts.ready;
  const loaded = face.loaded;
  globalThis.__fontFaceBinarySourceProbe = {
    source: face.source,
    status: face.status,
    samePromise: face.load() === loaded,
    readyChanged: fonts.ready !== readyBefore,
    setStatus: fonts.status,
    events,
    rejection: 'pending'
  };
  loaded.then(
    () => { globalThis.__fontFaceBinarySourceProbe.rejection = 'resolved'; },
    error => { globalThis.__fontFaceBinarySourceProbe.rejection = error.name; }
  );
})()
"#,
    )
    .expect("invalid binary FontFace source should initialize");

    let result = vm
        .eval("JSON.stringify(globalThis.__fontFaceBinarySourceProbe)")
        .expect("invalid binary FontFace rejection should settle");
    assert_eq!(
        result,
        r#"{"source":"","status":"error","samePromise":true,"readyChanged":true,"setStatus":"loaded","events":["loading:0:true","loadingerror:1:true"],"rejection":"SyntaxError"}"#
    );
}
#[test]
fn font_face_set_bindings_share_event_target_and_validate_native_receivers() {
    let mut vm = new_storage_test_vm("https://font-face-set-bindings.test/");
    vm.eval(&format!(
            r#"(async () => {{
                {}
                const frame = (document.body || document.documentElement || document)
                    .appendChild(document.createElement('iframe'));
                const child = frame.contentWindow;
                const getter = Object.getOwnPropertyDescriptor(Document.prototype, 'fonts').get;
                const windowless = document.implementation.createHTMLDocument('');
                const mainConstructor = Object.getOwnPropertyDescriptor(window, 'FontFaceSet');
                const childConstructor = Object.getOwnPropertyDescriptor(child, 'FontFaceSet');
                const mainPrototype = FontFaceSet.prototype;
                const childPrototype = child.FontFaceSet.prototype;
                const poison = {{configurable: true, get() {{ throw new Error('author constructor'); }}}};
                let creation;
                try {{
                    Object.defineProperty(window, 'FontFaceSet', poison);
                    Object.defineProperty(child, 'FontFaceSet', poison);
                    const childFonts = getter.call(child.document);
                    creation = [
                        Object.getPrototypeOf(document.fonts) === mainPrototype,
                        Object.getPrototypeOf(windowless.fonts) === mainPrototype,
                        Object.getPrototypeOf(childFonts) === childPrototype,
                        childFonts === child.document.fonts,
                    ];
                }} finally {{
                    Object.defineProperty(window, 'FontFaceSet', mainConstructor);
                    Object.defineProperty(child, 'FontFaceSet', childConstructor);
                }}
                const main = await fontFaceSetBindingsProbe(window, document.fonts);
                const other = await fontFaceSetBindingsProbe(child, child.document.fonts);
                const parentSize = Object.getOwnPropertyDescriptor(FontFaceSet.prototype, 'size').get;
                const childSize = Object.getOwnPropertyDescriptor(child.FontFaceSet.prototype, 'size').get;
                const face = new FontFace('CrossRealm', 'url(unused.ttf)');
                child.FontFaceSet.prototype.add.call(document.fonts, face);
                FontFaceSet.prototype.add.call(child.document.fonts, face);
                const crossRealm = [parentSize.call(child.document.fonts), childSize.call(document.fonts),
                    child.FontFaceSet.prototype.has.call(document.fonts, face)];
                frame.remove();
                let illegal = false;
                try {{ new FontFaceSet([]); }} catch (error) {{ illegal = error instanceof TypeError; }}
                return JSON.stringify({{main, other, crossRealm, illegal, creation}});
            }})().then(
                result => globalThis.__fontFaceSetBindings = result,
                error => globalThis.__fontFaceSetBindings = String(error.stack || error)
            )"#,
            include_str!("../../../../../../tests/fixtures/fontfaceset-bindings.js"),
        ))
        .expect("FontFaceSet binding probe should evaluate");
    let result = vm
        .eval("__fontFaceSetBindings")
        .expect("FontFaceSet binding probe should settle");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    for realm in ["main", "other"] {
        assert_eq!(result[realm]["failures"], serde_json::json!([]), "{result}");
        assert_eq!(result[realm]["checks"], 221, "{result}");
    }
    assert_eq!(result["crossRealm"], serde_json::json!([1, 1, true]));
    assert_eq!(result["illegal"], true);
    assert_eq!(
        result["creation"],
        serde_json::json!([true, true, true, true])
    );
}
#[test]
fn font_face_set_declared_slots_ignore_prototype_spoofing() {
    let mut vm = new_storage_test_vm("https://font-face-set-declared-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const stringify = value => value === undefined ? 'undefined' : String(value);
  const descriptorShape = (prototype, receiver, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      typeof descriptor.get,
      descriptor.get && descriptor.get.name,
      descriptor.get && descriptor.get.length,
      typeof descriptor.set,
      descriptor.set && descriptor.set.name,
      descriptor.set && descriptor.set.length,
      descriptor.enumerable,
      descriptor.configurable,
      Object.prototype.hasOwnProperty.call(receiver, name)
    ].map(stringify).join(':');
  };
  const face = new FontFace('Demo', 'url(demo.woff)');
  const fonts = document.implementation.createHTMLDocument('').fonts;
  const style = document.createElement('style');
  style.textContent = '@font-face { font-family: "Spoofed"; src: url(spoof.woff); }';
  (document.head || document.documentElement || document).appendChild(style);
  let listenerCalls = 0;
  fonts.add(face);
  fonts.addEventListener('loading', () => { listenerCalls++; });
  fonts.dispatchEvent(new Event('loading'));
  const ownSlots = Object.getOwnPropertyNames(fonts)
    .filter(name => name.startsWith('__moliFontFaceSet'))
    .sort();

  FontFaceSet.prototype.__moliFontFaceSetFaces = [face];
  FontFaceSet.prototype.__moliFontFaceSetManualFaces = [face];
  FontFaceSet.prototype.__moliFontFaceSetConnectedFaces = [face];
  FontFaceSet.prototype.__moliFontFaceSetListeners = { loading: [() => { listenerCalls += 100; }] };
  FontFaceSet.prototype.__moliFontFaceSetStatus = 'loading';
  FontFaceSet.prototype.__moliFontFaceSetReady = Promise.resolve('spoofed');
  FontFaceSet.prototype.__moliFontFaceSetSize = 99;
  FontFaceSet.prototype.__moliFontFaceSetOwnerDocument = document;
  fonts.__moliFontFaceSetFaces = [];
  fonts.__moliFontFaceSetManualFaces = [];
  fonts.__moliFontFaceSetConnectedFaces = [];
  fonts.__moliFontFaceSetListeners = { loading: [() => { listenerCalls += 1000; }] };
  fonts.__moliFontFaceSetStatus = 'error';
  fonts.__moliFontFaceSetReady = Promise.resolve('ownSpoofed');
  fonts.__moliFontFaceSetSize = 0;
  fonts.__moliFontFaceSetOwnerDocument = null;

  const fake = Object.create(FontFaceSet.prototype);
  const documentFonts = document.fonts;
  const documentFontsSlots = Object.getOwnPropertyNames(documentFonts)
    .filter(name => name.startsWith('__moliFontFaceSet'))
    .sort();
  return JSON.stringify({
    real: [
      fonts.status,
      typeof fonts.ready.then,
      fonts.size,
      fonts.has(face),
      Array.from(fonts.values()).length,
      listenerCalls
    ].join('|'),
    fake: [
      () => fake.status,
      () => fake.size,
      () => FontFaceSet.prototype.has.call(fake, face),
      () => FontFaceSet.prototype.values.call(fake)
    ].map(callback => {
      try { callback(); return 'accepted'; }
      catch (error) { return error.name; }
    }).join('|'),
    documentFontsOwnerSlot: documentFontsSlots.includes('__moliFontFaceSetOwnerDocument'),
    documentFontsSlots,
    descriptors: ['status', 'ready', 'size']
      .map(name => descriptorShape(FontFaceSet.prototype, fonts, name)),
    hasOwnSize: Object.prototype.hasOwnProperty.call(fonts, 'size'),
    ownSlots
  });
})()
"#,
        )
        .expect("FontFaceSet declared slots should ignore prototype spoofing");

    assert_eq!(
        result,
        r#"{"real":"loaded|function|1|true|1|1","fake":"TypeError|TypeError|TypeError|TypeError","documentFontsOwnerSlot":false,"documentFontsSlots":[],"descriptors":["status:function:get status:0:undefined:undefined:undefined:true:true:false","ready:function:get ready:0:undefined:undefined:undefined:true:true:false","size:function:get size:0:undefined:undefined:undefined:true:true:false"],"hasOwnSize":false,"ownSlots":[]}"#
    );
}
#[test]
fn stylesheet_font_faces_use_intrinsics_after_public_constructor_replacement() {
    for materialize_first in [false, true] {
        for replacement in [
            "{ configurable: true, get() { calls++; throw new Error('public getter'); } }",
            "{ configurable: true, value: function() { calls++; throw new Error('public constructor'); } }",
        ] {
            let mut vm = new_storage_test_vm("https://font-face-intrinsic.test/");
            let result = vm.eval(&format!(r#"
(() => {{
  let calls = 0;
  const parent = document.body || document.documentElement || document;
  const frame = parent.appendChild(document.createElement('iframe'));
  return JSON.stringify([window, frame.contentWindow].map(w => {{
    const original = {materialize_first} ? w.FontFace : null;
    Object.defineProperty(w, 'FontFace', {replacement});
    const d = w.document, fonts = d.fonts;
    const style = d.createElement('style');
    style.textContent = '@font-face {{ font-family: OriginalFace; src: local(OriginalFace); }}';
    (d.head || d.documentElement || d).appendChild(style);
    const first = Array.from(fonts)[0];
    style.setAttribute('data-irrelevant', 'value');
    const stable = Array.from(fonts)[0] === first;
    style.sheet.insertRule('@font-face {{ font-family: Inserted; src: local(Inserted); }}', 1);
    const faces = Array.from(fonts);
    const result = {{
      families: faces.map(face => face.family),
      realm: faces.every(face => Object.getPrototypeOf(face).constructor instanceof w.Function),
      prototype: !original || faces.every(face => Object.getPrototypeOf(face) === original.prototype),
      stable: stable && faces[0] === first,
      collection: fonts === d.fonts,
      calls
    }};
    style.remove();
    result.removed = fonts.size === 0;
    return result;
  }}));
}})()
"#)).expect("CSS font faces should bypass replaced public constructors");
            let expected = serde_json::json!({
                "families": ["OriginalFace", "Inserted"], "realm": true, "prototype": true,
                "stable": true, "collection": true, "calls": 0, "removed": true,
            });
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&result).unwrap(),
                serde_json::json!([expected, expected]),
                "materialized: {materialize_first}, replacement: {replacement}"
            );
        }
    }
}

#[test]
fn document_fonts_tracks_connected_style_candidates() {
    let mut vm = new_storage_test_vm("https://document-fonts-candidates.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.head || document.documentElement || document;
  const fonts = document.fonts;
  const wrapper = document.createElement('section');
  const style = document.createElement('style');
  style.textContent = '@font-face { font-family: CandidateFace; src: url(candidate.woff2); }';
  wrapper.appendChild(style);
  const initial = fonts.size;
  parent.appendChild(wrapper);
  const connected = fonts.size;
  wrapper.remove();
  const removed = fonts.size;
  parent.appendChild(wrapper);
  const reconnected = fonts.size;
  return [initial, connected, removed, reconnected].join('|');
})()
"#,
        )
        .expect("document FontFaceSet should follow connected style candidates");

    assert_eq!(result, "0|1|0|1");
}
#[test]
fn document_fonts_updates_only_the_changed_owner_contribution() {
    let mut vm = new_storage_html_test_vm("https://document-fonts-owner-projection.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.head || document.documentElement || document;
  const first = document.createElement('style');
  const second = document.createElement('style');
  first.textContent = '@font-face { font-family: FirstFace; src: url(first.woff2); }';
  second.textContent = '@font-face { font-family: SecondFace; src: url(second.woff2); }';
  parent.appendChild(first);
  parent.appendChild(second);
  const fonts = document.fonts;
  const byFamily = () => new Map(Array.from(fonts, face => [face.family, face]));
  const initial = byFamily();

  first.setAttribute('data-state', 'irrelevant');
  const afterIrrelevant = byFamily();

  first.textContent = '@font-face { font-family: ReplacementFace; src: url(replacement.woff2); }';
  const afterContents = byFamily();

  second.setAttribute('type', 'text/plain');
  const afterType = byFamily();

  return [
    initial.size,
    afterIrrelevant.get('FirstFace') === initial.get('FirstFace'),
    afterIrrelevant.get('SecondFace') === initial.get('SecondFace'),
    afterContents.has('FirstFace'),
    afterContents.has('ReplacementFace'),
    afterContents.get('SecondFace') === initial.get('SecondFace'),
    afterType.has('SecondFace'),
    afterType.size,
  ].join('|');
})()
"#,
        )
        .expect("document FontFaceSet should update one owner contribution at a time");

    assert_eq!(result, "2|true|true|false|true|true|false|1");
}
#[test]
fn linked_source_install_projects_font_faces_before_its_load_event() {
    let mut vm = new_storage_test_vm("https://linked-font-projection.test/");
    let request_url = url::Url::parse("https://linked-font-projection.test/fonts.css").unwrap();
    let initial = vm
        .eval(
            r#"
(() => {
  const parent = document.head || document.documentElement || document;
  const fonts = document.fonts;
  const link = document.createElement('link');
  link.id = 'linked-font-owner';
  link.rel = 'stylesheet';
  link.href = '/fonts.css';
  parent.appendChild(link);
  return fonts.size;
})()
"#,
        )
        .expect("linked font projection setup should evaluate");
    let owner = native_element_handle_by_id(&vm, "linked-font-owner");
    install_linked_stylesheet_for_test(
        &mut vm,
        owner,
        request_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(
            "@font-face { font-family: LinkedFace; src: url(linked.woff2); }".to_owned(),
            request_url.clone(),
        )
        .with_sheet_url(request_url),
    );
    let result = vm
        .eval(
            r#"
(() => {
  const fonts = document.fonts;
  const installed = Array.from(fonts, face => face.family).join(',');
  document.getElementById('linked-font-owner').remove();
  return [installed, fonts.size].join('|');
})()
"#,
        )
        .expect("linked font source projection should be live before event dispatch");

    assert_eq!(format!("{initial}|{result}"), "0|LinkedFace|0");
}
#[test]
fn inline_cssom_rule_edits_project_font_faces_synchronously() {
    let mut vm = new_storage_test_vm("https://cssom-font-projection.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.head || document.documentElement || document;
  const style = document.createElement('style');
  parent.appendChild(style);
  const fonts = document.fonts;
  const initial = fonts.size;
  style.sheet.insertRule('@font-face { font-family: CssomFace; src: url(cssom.woff2); }', 0);
  const inserted = [fonts.size, Array.from(fonts, face => face.family).join(',')].join(':');
  style.sheet.deleteRule(0);
  return [initial, inserted, fonts.size].join('|');
})()
"#,
        )
        .expect("inline CSSOM font-face edits should project before the next JS read");

    assert_eq!(result, "0|1:CssomFace|0");
}
#[test]
fn cssom_mutation_burst_defers_font_face_projection_until_observation() {
    let mut vm = new_storage_test_vm("https://cssom-font-projection-batch.test/");

    let initial = vm
        .eval(
            r#"
(() => {
  const parent = document.head || document.documentElement || document;
  const style = document.createElement('style');
  style.textContent = `
    @font-face { font-family: BatchedFace; src: local("Arial"); }
    .item-0 { color: red; }
  `;
  parent.appendChild(style);
  globalThis.__batchedFontSheet = style.sheet;
  return document.fonts.size;
})()
"#,
        )
        .expect("font projection setup should evaluate");
    assert_eq!(initial, "1");

    crate::live_stylesheet::reset_live_stylesheet_font_face_projection_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  for (let i = 1; i <= 1000; i += 1) {
    globalThis.__batchedFontSheet.cssRules[1].selectorText = `.item-${i}`;
  }
  return [document.fonts.size, document.fonts.size].join(':');
})()
"#,
        )
        .expect("batched stylesheet mutations should evaluate");
    assert_eq!(result, "1:1");
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_font_face_projection_count_for_test(),
        1
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__batchedFontSheet.cssRules[0].style.fontWeight = '700';
  return document.fonts.size;
})()
"#,
        )
        .expect("font descriptor mutation should evaluate");
    assert_eq!(result, "1");
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_font_face_projection_count_for_test(),
        2
    );
}
#[test]
fn inline_stylesheet_font_faces_follow_native_rule_and_sheet_identity() {
    let mut vm = new_storage_test_vm("https://inline-font-face-identity.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.head || document.documentElement || document;
  const style = document.createElement('style');
  const css = '@font-face { font-family: InlineIdentity; src: local("Arial"); }';
  style.textContent = css;
  parent.appendChild(style);
  const byFamily = () => [...document.fonts]
    .find(face => face.family === 'InlineIdentity');

  const initial = byFamily();
  initial.marker = 'initial';
  style.sheet.insertRule('body { color: red; }', style.sheet.cssRules.length);
  const afterUnrelatedInsert = byFamily();

  style.sheet.cssRules[0].style.fontWeight = '700';
  const afterDescriptor = byFamily();
  afterDescriptor.marker = 'descriptor';

  style.media = 'print';
  const hiddenSize = document.fonts.size;
  style.media = 'screen';
  const afterMediaRestore = byFamily();
  afterMediaRestore.marker = 'restored';

  style.textContent = css;
  const afterOwnerReplacement = byFamily();
  return JSON.stringify({
    unrelatedInsertIdentity:
      afterUnrelatedInsert === initial && afterUnrelatedInsert.marker === 'initial',
    descriptorChangedIdentity:
      afterDescriptor !== afterUnrelatedInsert && afterDescriptor.marker === 'descriptor',
    hiddenSize,
    mediaRestoreChangedIdentity:
      afterMediaRestore !== afterDescriptor && afterMediaRestore.marker === 'restored',
    ownerReplacementChangedIdentity:
      afterOwnerReplacement !== afterMediaRestore && afterOwnerReplacement.marker === undefined,
  });
})()
"#,
        )
        .expect("inline stylesheet FontFace identity should follow Chromium");

    assert_eq!(
        result,
        r#"{"unrelatedInsertIdentity":true,"descriptorChangedIdentity":true,"hiddenSize":0,"mediaRestoreChangedIdentity":true,"ownerReplacementChangedIdentity":true}"#
    );
}
#[test]
fn inline_stylesheet_copy_on_write_preserves_unmodified_font_face_wrappers() {
    let mut vm = new_storage_test_vm("https://inline-font-face-copy-on-write.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const parent = document.head || root.appendChild(document.createElement('head'));
  const css = '@font-face { font-family: SharedInlineFace; src: local("Arial"); }';
  const firstOwner = document.createElement('style');
  const secondOwner = document.createElement('style');
  firstOwner.textContent = css;
  secondOwner.textContent = css;
  parent.append(firstOwner, secondOwner);

  const faces = () => [...document.fonts]
    .filter(face => face.family === 'SharedInlineFace');
  const initial = faces();
  initial[0].marker = 'first';
  initial[1].marker = 'second';

  firstOwner.sheet.insertRule('body { color: red; }', 1);
  const afterUnrelatedInsert = faces();
  firstOwner.sheet.cssRules[0].style.fontWeight = '700';
  const afterDescriptorMutation = faces();

  return JSON.stringify({
    initialCount: initial.length,
    insertPreservedFirst:
      afterUnrelatedInsert[0] === initial[0] && afterUnrelatedInsert[0].marker === 'first',
    insertPreservedSecond:
      afterUnrelatedInsert[1] === initial[1] && afterUnrelatedInsert[1].marker === 'second',
    descriptorReplacedFirst:
      afterDescriptorMutation[0] !== afterUnrelatedInsert[0] &&
      afterDescriptorMutation[0].marker === undefined,
    descriptorPreservedSecond:
      afterDescriptorMutation[1] === afterUnrelatedInsert[1] &&
      afterDescriptorMutation[1].marker === 'second',
  });
})()
"#,
        )
        .expect("inline stylesheet COW should preserve native FontFace identity");

    assert_eq!(
        result,
        r#"{"initialCount":2,"insertPreservedFirst":true,"insertPreservedSecond":true,"descriptorReplacedFirst":true,"descriptorPreservedSecond":true}"#
    );
}

#[test]
fn css_owner_changes_prepare_font_projections_only_for_exposed_collections() {
    use crate::native_bridge::document::take_owner_font_face_projection_count_for_test;
    let mut vm = new_storage_test_vm("https://lazy-font-projection.test/");
    take_owner_font_face_projection_count_for_test();
    vm.eval(
        r#"
      globalThis.lazyStyle = document.createElement('style');
      (document.head || document.documentElement || document).appendChild(lazyStyle);
      globalThis.heldSheets = document.styleSheets;
      for (let i = 0; i < 64; i++) {
        lazyStyle.textContent = `@font-face { font-family: Face${i}; src: local(Face${i}); }
          #styled { color: rgb(1, 2, 3); }`;
      }
      globalThis.styled = document.createElement('div'); styled.id = 'styled';
      (document.body || document.documentElement || document).appendChild(styled);
      'ready'
    "#,
    )
    .unwrap();
    assert_eq!(
        take_owner_font_face_projection_count_for_test(),
        0,
        "unobserved FontFaceSets must not prepare owner projections"
    );
    assert_eq!(
        vm.eval("[heldSheets.length, getComputedStyle(styled).color].join('|')")
            .unwrap(),
        "1|rgb(1, 2, 3)"
    );
    assert_eq!(
        take_owner_font_face_projection_count_for_test(),
        0,
        "native styling and exposed StyleSheetLists do not consume JS font projections"
    );
    assert_eq!(vm.eval("globalThis.heldFonts = document.fonts; Array.from(heldFonts, face => face.family).join(',')").unwrap(), "Face63");
    assert_eq!(
        take_owner_font_face_projection_count_for_test(),
        1,
        "first observation prepares the current native state once"
    );
    assert_eq!(
        vm.eval(
            r#"
      lazyStyle.textContent = '@font-face { font-family: UpdatedFace; src: local(UpdatedFace); }';
      Array.from(heldFonts, face => face.family).join(',')
    "#
        )
        .unwrap(),
        "UpdatedFace"
    );
    assert!(
        take_owner_font_face_projection_count_for_test() > 0,
        "already held FontFaceSets must update without rereading Document.fonts"
    );
    assert_eq!(vm.eval("lazyStyle.remove(); [heldFonts.size, heldSheets.length, heldFonts === document.fonts].join('|')").unwrap(), "0|0|true");
}

#[test]
fn font_face_rejects_malformed_string_and_magic_prefixed_payloads() {
    let mut vm = new_storage_test_vm("https://font-face-payload-validation.test/");

    vm.eval(
        r#"
(() => {
  const invalidSource = new FontFace('InvalidSource', 'garbage');
  const invalidUrl = new FontFace(
    'InvalidUrl',
    'url("data:font/woff2;base64,d09GMmdhcmJhZ2U=")'
  );
  const invalidBytes = new FontFace(
    'InvalidBytes',
    new TextEncoder().encode('wOF2garbage')
  );
  globalThis.__fontFaceMalformedProbe = {
    initial: [invalidSource.status, invalidUrl.status, invalidBytes.status],
    settled: ['pending', 'pending', 'pending']
  };
  [invalidSource, invalidUrl, invalidBytes].forEach((face, index) => {
    face.load().then(
      () => { __fontFaceMalformedProbe.settled[index] = 'resolved'; },
      error => { __fontFaceMalformedProbe.settled[index] = error.name; }
    );
  });
})()
"#,
    )
    .expect("malformed FontFace payload probe should initialize");

    let result = vm
        .eval("JSON.stringify(globalThis.__fontFaceMalformedProbe)")
        .expect("malformed FontFace payload promises should settle");
    assert_eq!(
        result,
        r#"{"initial":["error","error","error"],"settled":["SyntaxError","NetworkError","SyntaxError"]}"#
    );
}
