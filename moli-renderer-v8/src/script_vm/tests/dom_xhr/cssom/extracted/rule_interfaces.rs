use super::*;

#[test]
fn attached_rule_sync_writes_snapshot_only_when_the_rule_detaches() {
    crate::context_bootstrap::css_stylesheet_runtime::reset_detached_css_rule_snapshot_write_count_for_test();
    let mut vm = new_storage_test_vm("https://css-attached-rule-snapshot.test/");

    let attached_text = vm
        .eval(
            r#"
globalThis.__snapshotSheet = new CSSStyleSheet();
__snapshotSheet.replaceSync('.before { color: red; }');
globalThis.__snapshotRule = __snapshotSheet.cssRules[0];
__snapshotRule.selectorText = '.after';
__snapshotRule.style.color = 'blue';
__snapshotRule.cssText = 'not a valid style rule';
__snapshotRule.cssText;
"#,
        )
        .expect("attached CSS rule mutation should evaluate");
    assert_eq!(attached_text, ".after { color: blue; }");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::detached_css_rule_snapshot_write_count_for_test(),
        0,
        "attached synchronization must not retain a full cssText snapshot"
    );

    let detached_text = vm
        .eval("__snapshotSheet.deleteRule(0); __snapshotRule.cssText;")
        .expect("detaching the CSS rule should freeze its current native state");
    assert_eq!(detached_text, ".after { color: blue; }");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::detached_css_rule_snapshot_write_count_for_test(),
        1,
        "detach should freeze exactly one snapshot for the retained wrapper"
    );

    let writes_before_detached_mutation =
        crate::context_bootstrap::css_stylesheet_runtime::detached_css_rule_snapshot_write_count_for_test();
    let mutated_text = vm
        .eval("__snapshotRule.style.color = 'green'; __snapshotRule.cssText;")
        .expect("detached CSS rule should remain independently mutable");
    assert_eq!(mutated_text, ".after { color: green; }");
    assert!(
        crate::context_bootstrap::css_stylesheet_runtime::detached_css_rule_snapshot_write_count_for_test()
            > writes_before_detached_mutation
    );
}
#[test]
fn css_rule_type_getter_uses_attached_native_rule_after_reset() {
    let mut vm = new_storage_test_vm("https://css-rule-type-live-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@supports (display: grid) { @media screen { .old { color: red; } } } .after { color: black; }');
  const supports = sheet.cssRules[0];
  const supportsRules = supports.cssRules;

  supports.cssText = '@supports (display: flex) { @container card (min-width: 10px) { .new { margin: 0; } } }';
  const container = supports.cssRules[0];
  const child = container.cssRules[0];
  return [
    supports.type,
    CSSRule.SUPPORTS_RULE,
    container.type,
    CSSRule.CONTAINER_RULE,
    child.type,
    CSSRule.STYLE_RULE,
    supports.cssRules === supportsRules,
    sheet.cssRules[0].type,
    sheet.cssRules[0].cssText === supports.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSRule.type should use the attached native rule after cssText reset");

    assert_eq!(result, "12|12|17|17|1|1|true|12|true");
}
#[test]
fn css_supported_at_rule_public_getters_use_attached_native_rules() {
    let mut vm = new_storage_test_vm("https://css-at-rule-public-getters-live-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.textContent = `
    @import url("support/a.css") layer(old) supports((display: flex)) screen;
    @namespace svg url("http://www.w3.org/2000/svg");
    @layer alpha { .layered { color: red; } }
    @media screen { .media { color: red; } }
    @supports (display: grid) { .supports { display: grid; } }
    @page :first { margin-top: 1px; @top-left { content: "x"; } }
    @property --accent { syntax: "<color>"; inherits: false; initial-value: red; }
    @counter-style thumbs { system: cyclic; symbols: "*"; suffix: " "; }
    @font-feature-values old_family { @annotation { mark: 1; } }
    @keyframes fade { from { opacity: 0; } to { opacity: 1; } }
  `;
  head.appendChild(style);
  const sheet = style.sheet;
  const [
    importRule,
    namespaceRule,
    layerRule,
    mediaRule,
    supportsRule,
    pageRule,
    propertyRule,
    counterRule,
    fontFeatureRule,
    keyframesRule
  ] = Array.from(sheet.cssRules);

  const importMedia = importRule.media;
  const mediaList = mediaRule.media;
  const pageRules = pageRule.cssRules;
  const fontAnnotation = fontFeatureRule.annotation;
  const keyframeRules = keyframesRule.cssRules;

  importRule.media.mediaText = 'print and (WiDtH)';
  mediaRule.media.mediaText = 'speech';
  supportsRule.cssText = '@supports (display: flex) { .supports { display: flex; } }';
  pageRule.selectorText = ':left';
  propertyRule.cssText = '@property --tone { syntax: "*"; inherits: true; }';
  counterRule.cssText = '@counter-style dots { system: cyclic; symbols: "."; suffix: " "; }';
  fontFeatureRule.fontFamily = 'new_family';
  keyframesRule.name = 'slide';
  const index = sheet.insertRule('.temp { color: green; }', sheet.cssRules.length);
  sheet.deleteRule(index);

  return [
    sheet.cssRules.length === 10,
    sheet.cssRules[0] === importRule,
    importRule.media === importMedia,
    importRule.href.includes('support/a.css'),
    importRule.media.mediaText === 'print and (width)',
    importRule.layerName === 'old',
    importRule.supportsText === '(display: flex)',
    namespaceRule.prefix === 'svg',
    namespaceRule.namespaceURI === 'http://www.w3.org/2000/svg',
    layerRule.name === 'alpha',
    layerRule.cssRules[0].selectorText === '.layered',
    mediaRule.media === mediaList,
    mediaRule.media.mediaText === 'speech',
    supportsRule.conditionText === '(display: flex)',
    pageRule.selectorText === ':left',
    pageRule.cssRules === pageRules,
    pageRule.cssRules[0].name === 'top-left',
    pageRule.style.marginTop === '1px',
    propertyRule.name === '--tone',
    propertyRule.syntax === '*',
    propertyRule.inherits === true,
    propertyRule.initialValue === null,
    counterRule.name === 'dots',
    fontFeatureRule.fontFamily === 'new_family',
    fontFeatureRule.annotation === fontAnnotation,
    fontFeatureRule.annotation.get('mark').join(',') === '1',
    keyframesRule.name === 'slide',
    keyframesRule.cssRules === keyframeRules,
    keyframesRule.findRule('from').style.opacity === '0',
    sheet.cssRules[9].cssText === keyframesRule.cssText
  ].join('|');
})()
"#,
        )
        .expect("supported at-rule public getters should use attached native rules");

    assert_eq!(
        result,
        "true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true"
    );
}
#[test]
fn cssom_rule_mutation_refreshes_computed_style_without_generation_bump() {
    let mut vm = new_storage_test_vm("https://cssom-rule-source-revision.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.id = 'target';
  globalThis.__sourceRevisionSheet = new CSSStyleSheet();
  globalThis.__sourceRevisionSheet.replaceSync('#target { color: rgb(1, 2, 3); }');
  document.adoptedStyleSheets = [globalThis.__sourceRevisionSheet];
  globalThis.__sourceRevisionStyle = getComputedStyle(target);
  return globalThis.__sourceRevisionStyle.color;
})()
"#,
        )
        .expect("CSSOM source revision setup should evaluate");
    assert_eq!(initial, "rgb(1, 2, 3)");
    let generation_after_setup = vm.computed_style_cache_generation_for_document_for_test(document);

    let same = vm
        .eval(
            r#"
(() => {
  const rule = globalThis.__sourceRevisionSheet.cssRules[0];
  rule.style.setProperty('color', 'rgb(1, 2, 3)');
  return globalThis.__sourceRevisionStyle.color;
})()
"#,
        )
        .expect("CSSOM no-op rule mutation should evaluate");
    assert_eq!(same, "rgb(1, 2, 3)");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_after_setup
    );

    let changed = vm
        .eval(
            r#"
(() => {
  const rule = globalThis.__sourceRevisionSheet.cssRules[0];
  rule.style.setProperty('color', 'rgb(4, 5, 6)');
  return globalThis.__sourceRevisionStyle.color;
})()
"#,
        )
        .expect("CSSOM changed rule mutation should evaluate");
    assert_eq!(changed, "rgb(4, 5, 6)");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_after_setup
    );
}
#[test]
fn css_nested_style_assignment_preserves_existing_nested_rules() {
    let mut vm = new_storage_test_vm("https://css-nested-style-assignment.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.a { color: red; & .b { color: green; } & .c { color: blue; } }');
  const rule = sheet.cssRules[0];
  rule.insertRule('@supports selector(&) { & div { font-size: 10px; }}', 1);
  rule.style = 'color: olivedrab; &.d { color: peru; }';
  return [
    rule.cssRules.length,
    rule.cssRules[0].cssText,
    rule.cssRules[1].cssText,
    rule.cssRules[2].cssText,
    rule.cssText,
  ].join('|');
})()
"#,
        )
        .expect("style assignment should preserve existing nested rules");

    assert_eq!(
        result,
        "3|& .b { color: green; }|@supports selector(&) {\n  & div { font-size: 10px; }\n}|& .c { color: blue; }|.a {\n  color: olivedrab;\n  & .b { color: green; }\n  @supports selector(&) {\n  & div { font-size: 10px; }\n}\n  & .c { color: blue; }\n}"
    );
}
#[test]
fn child_window_exposes_cssom_rule_constructors_for_inserted_rules() {
    let mut vm = new_storage_test_vm("https://css-rule-child-realm.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const iframe = document.createElement('iframe');
  body.appendChild(iframe);
  const doc = iframe.contentDocument;
  const head = doc.head || doc.documentElement.appendChild(doc.createElement('head'));
  const style = doc.createElement('style');
  head.appendChild(style);
  const sheet = style.sheet;
  style.remove();
  sheet.insertRule('.kaoru {}');
  const constructed = new iframe.contentWindow.CSSStyleSheet();
  constructed.insertRule('.kaoru {}');
  return [
    typeof iframe.contentWindow.CSSStyleRule,
    sheet.cssRules[0].constructor === iframe.contentWindow.CSSStyleRule,
    constructed.cssRules[0].constructor === iframe.contentWindow.CSSStyleRule
  ].join('|');
})()
"#,
        )
        .expect("child CSSOM rule constructors should evaluate");

    assert_eq!(result, "function|true|true");
}
#[test]
fn css_page_descriptor_setters_use_stylo_descriptor_entries() {
    let mut vm = new_storage_test_vm("https://css-page-descriptor-entry-setter.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page { margin-top: 1px; size: portrait; }');
  const page = sheet.cssRules[0];
  const style = page.style;

  style.setProperty('margin', '2px 4px', 'important');
  const marginPart = [
    style.length,
    style.item(0),
    style.item(1),
    style.item(2),
    style.item(3),
    style.item(4),
    style.margin,
    style.marginTop,
    style.marginRight,
    style.marginBottom,
    style.marginLeft,
    style.getPropertyPriority('margin'),
    style.getPropertyPriority('margin-left'),
    style.cssText.includes('margin: 2px 4px !important'),
    page.cssText.includes('margin: 2px 4px !important')
  ].join(',');

  style.size = 'landscape';
  style.pageOrientation = 'rotate-left';
  style.size = 'definitely-invalid';
  style.setProperty('page-orientation', 'rotate-right !important');
  style.marks = 'crop';
  const descriptorPart = [
    style.size,
    style.pageOrientation,
    style.marks,
    style.cssText.includes('size: landscape'),
    style.cssText.includes('page-orientation: rotate-left'),
    page.cssText.includes('size: landscape'),
    page.cssText.includes('page-orientation: rotate-left')
  ].join(',');

  const removed = style.removeProperty('margin');
  const removePart = [
    removed,
    style.length,
    style.margin,
    style.marginTop,
    style.marginRight,
    style.marginBottom,
    style.marginLeft,
    style.getPropertyPriority('margin'),
    style.cssText.includes('margin-')
  ].join(',');

  return [marginPart, descriptorPart, removePart].join('|');
})()
"#,
        )
        .expect("CSSPageDescriptors setters should use Stylo descriptor entries");

    assert_eq!(
        result,
        "5,size,margin-top,margin-right,margin-bottom,margin-left,2px 4px,2px,4px,2px,4px,important,important,true,true|landscape,rotate-left,,true,true,true,true|2px 4px,2,,,,,,,false"
    );
}
#[test]
fn css_page_and_margin_invalid_style_mutation_stays_stylo_canonical() {
    let mut vm = new_storage_test_vm("https://css-page-margin-invalid-style-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page :first { margin-top: 1px; @bottom-right { content: "x"; color: red; } } .after { color: black; }');
  const page = sheet.cssRules[0];
  const pageStyle = page.style;
  const margin = page.cssRules[0];
  const marginStyle = margin.style;
  const beforePage = page.cssText;
  const beforeMargin = margin.cssText;

  pageStyle.marginTop = '1px; margin-bottom: 2px';
  pageStyle.setProperty('size', 'notarealsize');
  pageStyle.cssText = 'margin-top: 10px; size: definitely-invalid;';
  marginStyle.setProperty('content', '"y"; color: blue');
  marginStyle.cssText = 'content: "y"; color: notacolor;';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === page,
    page.style === pageStyle,
    page.cssRules[0] === margin,
    margin.style === marginStyle,
    page.style.marginTop,
    margin.name,
    margin.style.getPropertyValue('content'),
    margin.style.color,
    page.cssText.includes('@bottom-right'),
    margin.cssText.startsWith('@bottom-right'),
    !page.cssText.includes('@top-left'),
    page.cssText.includes('margin-top: 10px'),
    !page.cssText.includes('definitely-invalid'),
    !page.cssText.includes('notarealsize'),
    !page.cssText.includes('color: notacolor'),
    !page.cssText.includes('color: blue'),
    !page.cssText.includes('margin-bottom: 2px'),
    page.cssText !== beforePage,
    margin.cssText !== beforeMargin,
    sheet.cssRules[0].cssText === page.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("invalid CSSPageRule/CSSMarginRule style mutation should stay Stylo-canonical");

    assert_eq!(
        result,
        r#"2|true|true|true|true|10px|bottom-right|"y"||true|true|true|true|true|true|true|true|true|true|true|true|.after { color: black; }"#
    );
}
#[test]
fn css_rule_css_text_uses_seeded_pdb_block_for_safe_rules_before_style_wrapper() {
    let mut vm = new_storage_test_vm("https://rule-style-seeded-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    div { display: block; opacity: 0.5; }
    span { --x: 1; display: block; }
    @keyframes fade {
      from { opacity: 0.5; }
      to { animation-name: spin; opacity: 1; }
    }
  `);
  const pure = sheet.cssRules[0];
  const mixed = sheet.cssRules[1];
  const keyframePure = sheet.cssRules[2].cssRules[0];
  const keyframeMixed = sheet.cssRules[2].cssRules[1];

  const beforeMaterializingStyle = [
    pure.cssText,
    keyframePure.cssText,
    mixed.cssText,
    keyframeMixed.cssText
  ].join('|');

  return [
    beforeMaterializingStyle,
    pure.style.cssText,
    keyframePure.style.cssText
  ].join('||');
})()
"#,
        )
        .expect("CSS rule cssText should use seeded PDB state when safe");

    assert_eq!(
        result,
        "div { display: block; opacity: 0.5; }|0% { opacity: 0.5; }|span { --x: 1; display: block; }|100% { opacity: 1; }||display: block; opacity: 0.5;||opacity: 0.5;"
    );
}
#[test]
fn css_rule_list_serialization_uses_rule_owned_pdb_block() {
    let mut vm = new_storage_test_vm("https://rule-list-pdb-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.id = 'pdb-sheet';
  head.appendChild(style);
  const sheet = style.sheet;
  sheet.insertRule('div { place-content: center start; }', 0);

  return sheet.cssRules[0].cssText;
})()
"#,
        )
        .expect("CSS rule list serialization should use rule-owned PDB block");

    assert_eq!(result, "div { place-content: center start; }");

    let style = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("pdb-sheet")
        .expect("style handle");
    let cached = vm._context_host.borrow().owner_style_sheet_text(style);
    assert_eq!(cached.as_deref(), Some(""));
    assert!(
        vm._context_host
            .borrow()
            .owner_live_stylesheet(style)
            .is_some_and(|stylesheet| {
                crate::style_engine::StyloStylesheetSource::from_live_stylesheet(&stylesheet)
                    .serialized_css_text()
                    .contains("place-content: center start")
            })
    );
}
#[test]
fn css_rule_mixed_style_serialization_uses_rule_owned_pdb_segment() {
    let mut vm = new_storage_test_vm("https://rule-mixed-pdb-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.id = 'mixed-pdb-sheet';
  head.appendChild(style);
  const sheet = style.sheet;
  sheet.insertRule('div {}', 0);
  const rule = sheet.cssRules[0];
  rule.style.cssText = '--before: one; width: 0; --middle: two; height: 0; --after: three;';

  return [
    rule.style.cssText,
    rule.cssText
  ].join('|');
})()
"#,
        )
        .expect("mixed CSS rule style serialization should use rule-owned PDB block segment");

    assert_eq!(
        result,
        "--before: one; width: 0px; --middle: two; height: 0px; --after: three;|div { --before: one; width: 0px; --middle: two; height: 0px; --after: three; }"
    );

    let style = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("mixed-pdb-sheet")
        .expect("style handle");
    let cached = vm._context_host.borrow().owner_style_sheet_text(style);
    assert_eq!(cached.as_deref(), Some(""));
    assert!(
        vm._context_host
            .borrow()
            .owner_live_stylesheet(style)
            .is_some_and(|stylesheet| {
                crate::style_engine::StyloStylesheetSource::from_live_stylesheet(&stylesheet)
                    .serialized_css_text()
                    .contains("--after: three")
            })
    );
}
#[test]
fn css_rule_mixed_style_mutation_snapshots_side_entries_once() {
    let mut vm = new_storage_test_vm("https://rule-mixed-pdb-snapshot.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('.subject {}');
  const rule = sheet.cssRules[0];
  for (let index = 0; index < 64; index += 1) {
    rule.style.setProperty(`--token-${index}`, String(index));
  }
  rule.style.setProperty('padding', '1px 2px');
  globalThis.__mixedSnapshotRule = rule;
})()
"#,
    )
    .expect("mixed CSS rule snapshot fixture should initialize");

    crate::detached_css_style::reset_raw_style_entries_snapshot_count_for_test();
    vm.eval("globalThis.__mixedSnapshotRule.style.setProperty('margin-left', '3px')")
        .expect("mixed CSS rule mutation should evaluate");
    let snapshot_count = crate::detached_css_style::raw_style_entries_snapshot_count_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const rule = globalThis.__mixedSnapshotRule;
  return [
    rule.style.getPropertyValue('--token-63'),
    rule.style.getPropertyValue('padding'),
    rule.style.getPropertyValue('margin-left'),
    rule.cssText.includes('--token-63: 63;')
  ].join('|');
})()
"#,
        )
        .expect("mixed CSS rule mutation should evaluate");

    assert_eq!(result, "63|1px 2px|3px|true");
    assert!(
        snapshot_count <= 3,
        "a mixed CSS rule mutation must reuse one side-entry snapshot per serialization; got {snapshot_count} snapshots"
    );
}
#[test]
fn css_rule_source_webkit_standard_aliases_seed_pdb_block() {
    let mut vm = new_storage_test_vm("https://css-rule-webkit-source-alias-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div { -webkit-transition: opacity 1s; -webkit-box-shadow: 1px 2px 3px red; -webkit-backface-visibility: visible; -webkit-background-clip: text; -webkit-transform-style: preserve-3d; }');
  const rule = sheet.cssRules[0];
  const before = [
    rule.style.length,
    rule.style.item(0),
    rule.style.getPropertyValue('transition'),
    rule.style.getPropertyValue('-webkit-transition'),
    rule.style.item(5),
    rule.style.getPropertyValue('box-shadow'),
    rule.style.getPropertyValue('-webkit-box-shadow'),
    rule.style.item(6),
    rule.style.getPropertyValue('backface-visibility'),
    rule.style.getPropertyValue('-webkit-backface-visibility'),
    rule.style.item(7),
    rule.style.getPropertyValue('background-clip'),
    rule.style.getPropertyValue('-webkit-background-clip'),
    rule.style.item(8),
    rule.style.getPropertyValue('transform-style'),
    rule.style.getPropertyValue('-webkit-transform-style'),
    rule.cssText
  ].join('|');
  const removed = rule.style.removeProperty('-webkit-transition');
  const after = [
    removed,
    rule.style.length,
    rule.style.getPropertyValue('transition'),
    rule.cssText
  ].join('|');
  return [before, after].join('||');
})()
"#,
        )
        .expect("rule source WebKit standard aliases should seed PDB block");

    assert_eq!(
        result,
        "9|transition-property|opacity 1s|opacity 1s|box-shadow|red 1px 2px 3px|red 1px 2px 3px|backface-visibility|visible|visible|background-clip|text|text|transform-style|preserve-3d|preserve-3d|div { transition: opacity 1s; box-shadow: red 1px 2px 3px; backface-visibility: visible; background-clip: text; transform-style: preserve-3d; }||opacity 1s|4||div { box-shadow: red 1px 2px 3px; backface-visibility: visible; background-clip: text; transform-style: preserve-3d; }"
    );
}
#[test]
fn css_namespace_to_string_tag_descriptor_matches_cssom() {
    let mut vm = new_storage_test_vm("https://css-namespace-tag.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptor = Object.getOwnPropertyDescriptor(CSS, Symbol.toStringTag);
  const before = [
    Object.prototype.hasOwnProperty.call(CSS, Symbol.toStringTag),
    descriptor.value,
    descriptor.writable,
    descriptor.enumerable,
    descriptor.configurable,
    Object.prototype.toString.call(CSS)
  ].join(',');
  Object.defineProperty(CSS, Symbol.toStringTag, { value: 'Other' });
  const afterDefine = Object.prototype.toString.call(CSS);
  const deleted = delete CSS[Symbol.toStringTag];
  return [before, afterDefine, deleted, Symbol.toStringTag in CSS].join('|');
})()
"#,
        )
        .expect("CSS namespace toStringTag descriptor should evaluate");

    assert_eq!(
        result,
        "true,CSS,false,false,true,[object CSS]|[object Other]|true|false"
    );
}
