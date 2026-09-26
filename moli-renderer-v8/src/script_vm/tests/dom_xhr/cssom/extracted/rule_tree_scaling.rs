use super::*;

#[test]
fn deleted_large_grouping_rule_keeps_detached_children_sparse() {
    let mut vm = new_storage_test_vm("https://css-deleted-large-rule-subtree.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  const children = Array.from(
    { length: 1000 },
    (_, index) => `.rule-${index} { --index: ${index}; }`
  ).join('\n');
  sheet.replaceSync(`@media screen { ${children} }`);
  const media = sheet.cssRules[0];
  const rules = media.cssRules;
  globalThis.__detachedLargeSheet = sheet;
  globalThis.__detachedLargeMedia = media;
  globalThis.__detachedLargeRules = rules;
  globalThis.__detachedLargeFirst = rules[0];
  globalThis.__detachedLargeMiddle = rules[500];
})()
"#,
    )
    .expect("large detached grouping fixture should initialize");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__detachedLargeSheet;
  const media = globalThis.__detachedLargeMedia;
  const rules = globalThis.__detachedLargeRules;
  const first = globalThis.__detachedLargeFirst;
  const middle = globalThis.__detachedLargeMiddle;
  sheet.deleteRule(0);
  const inserted = media.insertRule('.inserted { color: green; }', 1);
  return [
    sheet.cssRules.length,
    rules.length,
    inserted,
    first.parentStyleSheet === null,
    first.parentRule === media,
    middle.parentStyleSheet === null,
    rules[501] === middle,
  ].join('|');
})()
"#,
        )
        .expect("large detached grouping mutation should remain sparse");

    assert_eq!(result, "0|1001|1|true|true|true|true");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        0,
        "detach and detached insertion must not materialize untouched child wrappers"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const media = globalThis.__detachedLargeMedia;
  const rules = globalThis.__detachedLargeRules;
  const inserted = rules[1];
  const tail = rules[1000];
  return [
    inserted.cssText,
    inserted.parentStyleSheet === null,
    inserted.parentRule === media,
    tail.cssText,
    rules[1] === inserted,
    rules[1000] === tail,
  ].join('|');
})()
"#,
        )
        .expect("detached children should materialize on indexed access");

    assert_eq!(
        result,
        ".inserted { color: green; }|true|true|.rule-999 { --index: 999; }|true|true"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        2,
        "only explicitly accessed detached child wrappers should materialize"
    );
}
#[test]
fn deleted_large_keyframes_rule_keeps_detached_children_sparse() {
    let mut vm = new_storage_test_vm("https://css-deleted-large-keyframes.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  const frames = Array.from(
    { length: 1000 },
    (_, index) => `${(index / 10).toFixed(1)}% { --index: ${index}; }`
  ).join('\n');
  sheet.replaceSync(`@keyframes pulse { ${frames} }`);
  const keyframes = sheet.cssRules[0];
  const rules = keyframes.cssRules;
  globalThis.__detachedLargeKeyframesSheet = sheet;
  globalThis.__detachedLargeKeyframes = keyframes;
  globalThis.__detachedLargeKeyframeRules = rules;
  globalThis.__detachedLargeFirstFrame = rules[0];
  globalThis.__detachedLargeMiddleFrame = rules[500];
})()
"#,
    )
    .expect("large detached keyframes fixture should initialize");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__detachedLargeKeyframesSheet;
  const keyframes = globalThis.__detachedLargeKeyframes;
  const rules = globalThis.__detachedLargeKeyframeRules;
  const first = globalThis.__detachedLargeFirstFrame;
  const middle = globalThis.__detachedLargeMiddleFrame;
  sheet.deleteRule(0);
  keyframes.appendRule('100% { opacity: 1; }');
  return [
    sheet.cssRules.length,
    rules.length,
    first.parentStyleSheet === null,
    first.parentRule === keyframes,
    middle.parentStyleSheet === null,
    rules[500] === middle,
  ].join('|');
})()
"#,
        )
        .expect("large detached keyframes mutation should remain sparse");

    assert_eq!(result, "0|1001|true|true|true|true");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        0,
        "detach and detached appendRule must not materialize untouched keyframes"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const keyframes = globalThis.__detachedLargeKeyframes;
  const rules = globalThis.__detachedLargeKeyframeRules;
  const found = keyframes.findRule('99.9%');
  return [found === rules[999], found.cssText].join('|');
})()
"#,
        )
        .expect("detached keyframe lookup should materialize only the match");

    assert_eq!(result, "true|99.9% { --index: 999; }");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        1,
        "detached findRule must search snapshots without materializing preceding keyframes"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const keyframes = globalThis.__detachedLargeKeyframes;
  const rules = globalThis.__detachedLargeKeyframeRules;
  const tail = rules[999];
  const appended = rules[1000];
  return [
    tail.cssText,
    tail.parentStyleSheet === null,
    tail.parentRule === keyframes,
    appended.cssText,
    appended.parentStyleSheet === null,
    appended.parentRule === keyframes,
  ].join('|');
})()
"#,
        )
        .expect("detached keyframes should materialize on indexed access");

    assert_eq!(
        result,
        "99.9% { --index: 999; }|true|true|100% { opacity: 1; }|true|true"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        2,
        "only explicitly accessed detached keyframes should materialize"
    );
}
#[test]
fn css_supported_at_rule_materialization_uses_shallow_native_wrappers() {
    let mut vm = new_storage_test_vm("https://css-at-rule-materialization-stylo-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    @layer alpha, beta;
    @layer gamma { .x { margin: 0; } }
    @container card (min-width: 10px) { .y { color: red; } }
    @scope (.host) to (.end) { .z { display: block; } }
  `);
  const [statement, layer, container, scope] = Array.from(sheet.cssRules);
  const before = [
    sheet.cssRules.length,
    statement.constructor === CSSLayerStatementRule,
    statement instanceof CSSLayerStatementRule,
    statement.type,
    statement.nameList.join(','),
    Object.isFrozen(statement.nameList),
    layer.constructor === CSSLayerBlockRule,
    layer instanceof CSSLayerBlockRule,
    layer instanceof CSSGroupingRule,
    layer.type,
    layer.name,
    layer.cssRules[0].cssText,
    container.constructor === CSSContainerRule,
    container.type,
    container.containerName,
    container.conditionText,
    scope.constructor === CSSScopeRule,
    scope.start,
    scope.end
  ].join('|');

  sheet.insertRule('@layer delta { .d { padding: 1px; } }', sheet.cssRules.length);
  const inserted = sheet.cssRules[sheet.cssRules.length - 1];
  scope.cssText = '@scope (.fresh) { .fresh-rule { opacity: 1; } }';
  const after = [
    inserted.constructor === CSSLayerBlockRule,
    inserted.name,
    inserted.cssRules[0].cssText,
    scope.constructor === CSSScopeRule,
    scope.cssRules[0].cssText,
    sheet.cssRules[3].cssText === scope.cssText
  ].join('|');
  return `${before}||${after}`;
})()
"#,
        )
        .expect("supported at-rule materialization should use shallow native wrappers");

    assert_eq!(
        result,
        "4|true|true|0|alpha,beta|true|true|true|true|0|gamma|.x { margin: 0px; }|true|17|card|card (min-width: 10px)|true|.host|.end||true|delta|.d { padding: 1px; }|true|.fresh-rule { opacity: 1; }|true"
    );
}
#[test]
fn css_page_rule_public_mutations_use_attached_native_read_before_child_materialization() {
    let mut vm = new_storage_test_vm("https://css-page-public-mutation-stylo-source.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page :first { margin-top: 1px; @top-left { content: "x"; color: red; } } .after { color: black; }');
  const page = sheet.cssRules[0];
  const pageStyle = page.style;

  pageStyle.marginTop = '10px';
  page.selectorText = ':left';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  const rules = page.cssRules;
  const margin = rules[0];
  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === page,
    page.style === pageStyle,
    page.selectorText,
    pageStyle.marginTop,
    rules.length,
    margin instanceof CSSMarginRule,
    margin.name,
    margin.style.getPropertyValue('content'),
    margin.style.color,
    page.cssText.includes('@top-left'),
    sheet.cssRules[0].cssText === page.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSPageRule public mutations should use Stylo source before child materialization");

    assert_eq!(
        result,
        r#"2|true|true|:left|10px|1|true|top-left|"x"|red|true|true|.after { color: black; }"#
    );
}
#[test]
fn css_property_rule_materialization_uses_stylo_validation() {
    let mut vm = new_storage_test_vm("https://css-property-stylo-validation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const invalidProperty = '@property --bad { syntax: "<color>"; inherits: false; initial-value: 10px; }';
  const sheet = new CSSStyleSheet();
  let insertError = 'none';
  try {
    sheet.insertRule(invalidProperty, 0);
  } catch (error) {
    insertError = error.name;
  }

  sheet.replaceSync(`${invalidProperty} .ok { color: green; }`);
  return [
    insertError,
    sheet.cssRules.length,
    sheet.cssRules[0] instanceof CSSStyleRule,
    sheet.cssRules[0].cssText
  ].join('|');
})()
"#,
        )
        .expect("invalid property rules should be validated by Stylo");

    assert_eq!(result, "SyntaxError|1|true|.ok { color: green; }");
}
#[test]
fn css_rule_css_text_reads_do_not_project_the_entire_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://rule-css-text-local-projection.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    .first { color: red; }
    @media screen { .nested { color: blue; } }
    .last { color: green; }
  `);
  globalThis.__cssTextReadRules = [sheet.cssRules[0], sheet.cssRules[1]];
})()
"#,
    )
    .expect("CSS rule wrappers should materialize");

    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const [style, media] = globalThis.__cssTextReadRules;
  return [style.cssText, media.cssText, style.cssText, media.cssText].join('|');
})()
"#,
        )
        .expect("cached CSS rule cssText reads should evaluate");

    assert_eq!(
        result,
        ".first { color: red; }|@media screen {\n  .nested { color: blue; }\n}|.first { color: red; }|@media screen {\n  .nested { color: blue; }\n}"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0,
        "read-only cssText access must not serialize the containing stylesheet"
    );
}
#[test]
fn css_rule_wrappers_are_counted_at_the_materialization_boundary() {
    let mut vm = new_storage_test_vm("https://css-rule-wrapper-materialization-count.test/");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.textContent = Array.from(
    { length: 1000 },
    (_, index) => `.rule-${index} { --index: ${index}; }`
  ).join('\n');
  head.appendChild(style);
  const rules = style.sheet.cssRules;
  const length = rules.length;
  globalThis.__largeRuleList = rules;
  return length;
})()
"#,
        )
        .expect("large stylesheet should expose its rule count");

    assert_eq!(result, "1000");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        0,
        "reading CSSRuleList.length must not materialize any rule wrappers"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const rules = globalThis.__largeRuleList;
  const first = rules[0];
  const middle = rules.item(500);
  const last = Object.getOwnPropertyDescriptor(rules, "999").value;
  return [
    first.cssText,
    middle.cssText,
    last.cssText,
    rules[500] === middle,
    rules[0] === first,
  ].join('|');
})()
"#,
        )
        .expect("indexed CSSRuleList reads should materialize individual wrappers");

    assert_eq!(
        result,
        ".rule-0 { --index: 0; }|.rule-500 { --index: 500; }|.rule-999 { --index: 999; }|true|true"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        3,
        "repeated reads must preserve wrapper identity without materializing siblings"
    );
}
#[test]
fn css_rule_list_sparse_mutations_visit_only_materialized_entries() {
    let mut vm = new_storage_test_vm("https://css-rule-list-sparse-traversal.test/");

    for rule_count in [1_000_u32, 10_000_u32] {
        vm.eval(&format!(
            "globalThis.__sparseTraversalRuleCount = {rule_count};"
        ))
        .expect("sparse traversal rule count should initialize");
        vm.eval(
            r#"
(() => {
  const count = globalThis.__sparseTraversalRuleCount;
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(Array.from(
    { length: count },
    (_, index) => `.rule-${index} { --index: ${index}; }`
  ).join('\n'));
  const rules = sheet.cssRules;
  const middle = Math.floor(count / 2);
  globalThis.__sparseTraversalSheet = sheet;
  globalThis.__sparseTraversalRules = rules;
  globalThis.__sparseTraversalRetained = [rules[0], rules[middle], rules[count - 1]];
})()
"#,
        )
        .expect("sparse CSSRuleList traversal fixture should initialize");

        crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_list_materialized_traversal_metrics_for_test();
        let result = vm
            .eval(
                r#"
(() => {
  const count = globalThis.__sparseTraversalRuleCount;
  const sheet = globalThis.__sparseTraversalSheet;
  const rules = globalThis.__sparseTraversalRules;
  const retained = globalThis.__sparseTraversalRetained;
  const middle = Math.floor(count / 2);
  sheet.insertRule('.inserted { margin: 0; }', middle);
  return [
    rules.length,
    rules[0] === retained[0],
    rules[middle + 1] === retained[1],
    rules[count] === retained[2],
    rules[middle].cssText,
  ].join('|');
})()
"#,
            )
            .expect("middle insert should shift only materialized CSSRuleList entries");
        assert_eq!(
            result,
            format!(
                "{}|true|true|true|.inserted {{ margin: 0px; }}",
                rule_count + 1
            )
        );
        assert_eq!(
            crate::context_bootstrap::css_stylesheet_runtime::css_rule_list_materialized_traversal_metrics_for_test(),
            crate::context_bootstrap::css_stylesheet_runtime::CssRuleListMaterializedTraversalMetrics {
                traversals: 1,
                entries: 3,
            },
            "middle insert must not scan the logical CSSRuleList length"
        );

        crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_list_materialized_traversal_metrics_for_test();
        let result = vm
            .eval(
                r#"
(() => {
  const count = globalThis.__sparseTraversalRuleCount;
  const sheet = globalThis.__sparseTraversalSheet;
  const rules = globalThis.__sparseTraversalRules;
  const retained = globalThis.__sparseTraversalRetained;
  const middle = Math.floor(count / 2);
  sheet.deleteRule(middle);
  return [
    rules.length,
    rules[0] === retained[0],
    rules[middle] === retained[1],
    rules[count - 1] === retained[2],
  ].join('|');
})()
"#,
            )
            .expect("middle delete should shift only materialized CSSRuleList entries");
        assert_eq!(result, format!("{rule_count}|true|true|true"));
        assert_eq!(
            crate::context_bootstrap::css_stylesheet_runtime::css_rule_list_materialized_traversal_metrics_for_test(),
            crate::context_bootstrap::css_stylesheet_runtime::CssRuleListMaterializedTraversalMetrics {
                traversals: 1,
                entries: 4,
            },
            "middle delete must not scan the logical CSSRuleList length"
        );

        crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_list_materialized_traversal_metrics_for_test();
        let result = vm
            .eval(
                r#"
(() => {
  const sheet = globalThis.__sparseTraversalSheet;
  const rules = globalThis.__sparseTraversalRules;
  const retained = globalThis.__sparseTraversalRetained;
  sheet.replaceSync('.replacement { color: green; }');
  return [
    sheet.cssRules === rules,
    rules.length,
    retained[0].cssText,
    retained[1].cssText,
    retained[2].cssText,
  ].join('|');
})()
"#,
            )
            .expect("whole-sheet replacement should retire only materialized entries");
        assert_eq!(
            result,
            format!(
                "true|1|.rule-0 {{ --index: 0; }}|.rule-{} {{ --index: {}; }}|.rule-{} {{ --index: {}; }}",
                rule_count / 2,
                rule_count / 2,
                rule_count - 1,
                rule_count - 1,
            )
        );
        assert_eq!(
            crate::context_bootstrap::css_stylesheet_runtime::css_rule_list_materialized_traversal_metrics_for_test(),
            crate::context_bootstrap::css_stylesheet_runtime::CssRuleListMaterializedTraversalMetrics {
                traversals: 1,
                entries: 3,
            },
            "whole-sheet replacement must not scan the logical CSSRuleList length"
        );
    }
}
#[test]
fn css_rule_pdb_probe_visits_only_materialized_descendants() {
    let mut vm = new_storage_test_vm("https://css-rule-pdb-sparse-traversal.test/");

    for rule_count in [1_000_u32, 10_000_u32] {
        vm.eval(&format!(
            "globalThis.__pdbSparseTraversalRuleCount = {rule_count};"
        ))
        .expect("PDB sparse traversal rule count should initialize");
        vm.eval(
            r#"
(() => {
  const count = globalThis.__pdbSparseTraversalRuleCount;
  const sheet = new CSSStyleSheet();
  const children = Array.from(
    { length: count },
    (_, index) => `.rule-${index} { --index: ${index}; }`
  ).join('\n');
  sheet.replaceSync(`@media screen { ${children} }`);
  const media = sheet.cssRules[0];
  const rules = media.cssRules;
  const middle = Math.floor(count / 2);
  globalThis.__pdbSparseTraversalMedia = media;
  globalThis.__pdbSparseTraversalRules = rules;
  globalThis.__pdbSparseTraversalRetained = [rules[0], rules[middle], rules[count - 1]];
})()
"#,
        )
        .expect("PDB sparse traversal fixture should initialize");

        crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_list_materialized_traversal_metrics_for_test();
        let result = vm
            .eval(
                r#"
(() => {
  const count = globalThis.__pdbSparseTraversalRuleCount;
  const media = globalThis.__pdbSparseTraversalMedia;
  const rules = globalThis.__pdbSparseTraversalRules;
  const retained = globalThis.__pdbSparseTraversalRetained;
  const middle = Math.floor(count / 2);
  const text = media.cssText;
  return [
    text.startsWith('@media screen'),
    text.includes(`.rule-${count - 1} { --index: ${count - 1}; }`),
    rules[0] === retained[0],
    rules[middle] === retained[1],
    rules[count - 1] === retained[2],
  ].join('|');
})()
"#,
            )
            .expect("grouping cssText should probe only materialized PDB descendants");

        assert_eq!(result, "true|true|true|true|true");
        assert_eq!(
            crate::context_bootstrap::css_stylesheet_runtime::css_rule_list_materialized_traversal_metrics_for_test(),
            crate::context_bootstrap::css_stylesheet_runtime::CssRuleListMaterializedTraversalMetrics {
                traversals: 1,
                entries: 3,
            },
            "PDB side-entry detection must not scan the logical child-rule count"
        );
    }
}
#[test]
fn css_grouping_rule_children_materialize_on_indexed_access() {
    let mut vm = new_storage_test_vm("https://css-grouping-child-materialization.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  const children = Array.from(
    { length: 1000 },
    (_, index) => `.rule-${index} { --index: ${index}; }`
  ).join('\n');
  sheet.replaceSync(`@media screen { ${children} }`);
  globalThis.__largeGroupingSheet = sheet;
})()
"#,
    )
    .expect("large grouping stylesheet should parse");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__largeGroupingSheet;
  const media = sheet.cssRules[0];
  const rules = media.cssRules;
  globalThis.__largeGroupingRule = media;
  globalThis.__largeGroupingRuleList = rules;
  return [
    sheet.cssRules.length,
    media.conditionText,
    media.media.mediaText,
    rules.length,
  ].join('|');
})()
"#,
        )
        .expect("large grouping rule should expose native child count");

    assert_eq!(result, "1|screen|screen|1000");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        1,
        "reading a grouping rule child-list length must materialize only the parent wrapper"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test()
            .recursive_rule_snapshots,
        0,
        "materializing a grouping parent must not recursively project its child rules"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const rules = globalThis.__largeGroupingRuleList;
  const first = rules[0];
  const middle = rules.item(500);
  const last = Object.getOwnPropertyDescriptor(rules, '999').value;
  return [
    first.cssText,
    middle.cssText,
    last.cssText,
    rules[0] === first,
    rules[500] === middle,
  ].join('|');
})()
"#,
        )
        .expect("grouping child indexed reads should materialize individual wrappers");

    assert_eq!(
        result,
        ".rule-0 { --index: 0; }|.rule-500 { --index: 500; }|.rule-999 { --index: 999; }|true|true"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        4,
        "grouping child reads must preserve identity without materializing siblings"
    );
}
#[test]
fn css_page_rule_fields_do_not_project_large_margin_rule_subtree() {
    let mut vm = new_storage_test_vm("https://css-page-rule-shallow-read.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  const children = Array.from(
    { length: 1000 },
    (_, index) => `@top-left { --index: ${index}; }`
  ).join('\n');
  sheet.replaceSync(`@page :first { margin-top: 1px; ${children} }`);
  globalThis.__largePageRuleSheet = sheet;
})()
"#,
    )
    .expect("large page-rule stylesheet should parse");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const page = globalThis.__largePageRuleSheet.cssRules[0];
  return [
    page.selectorText,
    page.style.marginTop,
    page.cssRules.length,
  ].join('|');
})()
"#,
        )
        .expect("page rule fields should use shallow native reads");

    assert_eq!(result, ":first|1px|1000");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        1,
        "reading page fields and child-list length must materialize only the page wrapper"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test()
            .recursive_rule_snapshots,
        0,
        "page selector and declaration reads must not project margin-rule children"
    );
}
#[test]
fn css_nested_style_rule_children_materialize_without_recursive_projection() {
    let mut vm = new_storage_test_vm("https://css-style-rule-child-materialization.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  const children = Array.from(
    { length: 1000 },
    (_, index) => `& .rule-${index} { --index: ${index}; }`
  ).join('\n');
  sheet.replaceSync(`.host { color: red; ${children} }`);
  globalThis.__largeNestedStyleSheet = sheet;
})()
"#,
    )
    .expect("large nested style rule should parse");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__largeNestedStyleSheet;
  const parent = sheet.cssRules[0];
  const rules = parent.cssRules;
  globalThis.__largeNestedStyleRuleList = rules;
  return [
    parent.selectorText,
    parent.style.getPropertyValue('color'),
    rules.length,
  ].join('|');
})()
"#,
        )
        .expect("nested style parent should expose native declarations and child count");

    assert_eq!(result, ".host|red|1000");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        1,
        "reading a nested style child-list length must materialize only the parent wrapper"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test()
            .recursive_rule_snapshots,
        0,
        "materializing a nested style parent must not recursively project its child rules"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const rules = globalThis.__largeNestedStyleRuleList;
  const first = rules[0];
  const middle = rules.item(500);
  const last = Object.getOwnPropertyDescriptor(rules, '999').value;
  return [
    first.cssText,
    middle.cssText,
    last.cssText,
    rules[0] === first,
    rules[500] === middle,
  ].join('|');
})()
"#,
        )
        .expect("nested style child indexed reads should materialize individual wrappers");

    assert_eq!(
        result,
        "& .rule-0 { --index: 0; }|& .rule-500 { --index: 500; }|& .rule-999 { --index: 999; }|true|true"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        4,
        "nested style child reads must preserve identity without materializing siblings"
    );
}
#[test]
fn css_nested_declarations_materialize_without_rule_projection() {
    let mut vm = new_storage_test_vm("https://css-nested-declarations-materialization.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    @supports (display: grid) {
      .host {
        & .child { color: blue; }
        color: red;
        margin: 0;
      }
    }
  `);
  globalThis.__nestedDeclarationsSheet = sheet;
})()
"#,
    )
    .expect("nested declarations stylesheet should parse");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__nestedDeclarationsSheet;
  const supports = sheet.cssRules[0];
  const style = supports.cssRules[0];
  const declarations = style.cssRules[1];
  return [
    declarations instanceof CSSNestedDeclarations,
    declarations.cssText,
    declarations.style.color,
    declarations.style.margin,
    style.cssRules[1] === declarations,
  ].join('|');
})()
"#,
        )
        .expect("nested declarations wrapper should materialize from its native seed");

    assert_eq!(result, "true|color: red; margin: 0px;|red|0px|true");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        3,
        "only the explicitly read supports, style, and nested declarations wrappers should exist"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test()
            .recursive_rule_snapshots,
        0,
        "nested declarations materialization must not build a recursive rule snapshot"
    );
}
#[test]
fn css_keyframes_rule_children_materialize_on_indexed_access() {
    let mut vm = new_storage_test_vm("https://css-keyframes-child-materialization.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  const children = Array.from(
    { length: 1000 },
    (_, index) => `${index / 10}% { --index: ${index}; }`
  ).join('\n');
  sheet.replaceSync(`@keyframes dense { ${children} }`);
  globalThis.__largeKeyframesSheet = sheet;
})()
"#,
    )
    .expect("large keyframes stylesheet should parse");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__largeKeyframesSheet;
  const keyframes = sheet.cssRules[0];
  const rules = keyframes.cssRules;
  globalThis.__largeKeyframesRule = keyframes;
  globalThis.__largeKeyframesRuleList = rules;
  return [keyframes.name, keyframes.length, rules.length].join('|');
})()
"#,
        )
        .expect("large keyframes rule should expose native child count");

    assert_eq!(result, "dense|1000|1000");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        1,
        "reading keyframe counts must materialize only the CSSKeyframesRule wrapper"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test()
            .recursive_rule_snapshots,
        0,
        "materializing a keyframes parent must not recursively project its keyframe rules"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const keyframes = globalThis.__largeKeyframesRule;
  const rules = globalThis.__largeKeyframesRuleList;
  const found = keyframes.findRule('50%');
  return [found.cssText, found === rules[500]].join('|');
})()
"#,
        )
        .expect("attached keyframe lookup should search the native rule list");

    assert_eq!(result, "50% { --index: 500; }|true");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        2,
        "findRule must materialize only the matching keyframe"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const keyframes = globalThis.__largeKeyframesRule;
  const rules = globalThis.__largeKeyframesRuleList;
  const first = keyframes[0];
  const middle = rules.item(500);
  const last = Object.getOwnPropertyDescriptor(keyframes, '999').value;
  return [
    first.cssText,
    middle.cssText,
    last.cssText,
    keyframes[0] === rules[0],
    keyframes[500] === middle,
  ].join('|');
})()
"#,
        )
        .expect("keyframe indexed reads should share lazy CSSRuleList wrappers");

    assert_eq!(
        result,
        "0% { --index: 0; }|50% { --index: 500; }|99.9% { --index: 999; }|true|true"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        4,
        "CSSKeyframesRule and cssRules must share sparse wrapper identity"
    );

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const keyframes = globalThis.__largeKeyframesRule;
  keyframes.deleteRule('25%');
  return [keyframes.length, keyframes.findRule('25%') === null].join('|');
})()
"#,
        )
        .expect("attached keyframe deletion should search the native rule list");

    assert_eq!(result, "999|true");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        0,
        "deleteRule must not materialize keyframes while locating the native match"
    );
}
#[test]
fn css_style_sheet_insert_rule_does_not_reproject_existing_rule_wrappers() {
    let mut vm = new_storage_test_vm("https://css-insert-rule-local-projection.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(Array.from(
    { length: 128 },
    (_, index) => `.rule-${index} { color: rgb(${index % 255}, 0, 0); }`
  ).join('\n'));
  globalThis.__insertRuleProjectionSheet = sheet;
  globalThis.__insertRuleProjectionFirst = sheet.cssRules[0];
  globalThis.__insertRuleProjectionMiddle = sheet.cssRules[64];
})()
"#,
    )
    .expect("CSS rule wrappers should materialize");

    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__insertRuleProjectionSheet;
  sheet.insertRule('.inserted { margin: 0; }', 64);
  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === globalThis.__insertRuleProjectionFirst,
    sheet.cssRules[65] === globalThis.__insertRuleProjectionMiddle,
    sheet.cssRules[64].cssText,
  ].join('|');
})()
"#,
        )
        .expect("incremental CSS rule insertion should evaluate");

    assert_eq!(result, "129|true|true|.inserted { margin: 0px; }");
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0,
        "insertRule must not reproject the live stylesheet after the mutation result"
    );
    let metrics = crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test();
    assert_eq!(metrics.native_top_level_mutations, 1);
    assert_eq!(
        metrics.recursive_rule_snapshots, 0,
        "materializing the inserted style rule must use its shallow native seed"
    );
}
#[test]
fn css_style_sheet_dense_append_mutations_stay_on_the_native_single_rule_path() {
    let mut vm = new_storage_test_vm("https://css-insert-rule-native-density.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('');
  globalThis.__denseNativeMutationSheet = sheet;
  globalThis.__denseNativeMutationRules = sheet.cssRules;
})()
"#,
    )
    .expect("dense native CSSStyleSheet fixture should initialize");

    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__denseNativeMutationSheet;
  const rules = globalThis.__denseNativeMutationRules;
  for (let index = 0; index < 1000; index++) {
    sheet.insertRule(`.rule-${index} { --index: ${index}; }`, rules.length);
  }
  return [
    rules.length,
    rules[0].cssText,
    rules[999].cssText,
  ].join('|');
})()
"#,
        )
        .expect("dense native CSSStyleSheet insertion should evaluate");

    assert_eq!(
        result,
        "1000|.rule-0 { --index: 0; }|.rule-999 { --index: 999; }"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0
    );
    let metrics = crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test();
    assert_eq!(metrics.native_top_level_mutations, 1000);
    assert_eq!(
        metrics.recursive_rule_snapshots, 0,
        "append-only insertRule must seed read wrappers without full rule projection"
    );
}
#[test]
fn css_grouping_rule_dense_append_avoids_full_stylesheet_projection() {
    let mut vm = new_storage_test_vm("https://css-grouping-rule-native-density.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { }');
  globalThis.__denseNestedMutationSheet = sheet;
  globalThis.__denseNestedMutationParent = sheet.cssRules[0];
  globalThis.__denseNestedMutationRules = sheet.cssRules[0].cssRules;
})()
"#,
    )
    .expect("dense native grouping-rule fixture should initialize");

    crate::context_bootstrap::css_stylesheet_runtime::reset_detached_rule_mutation_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let mutation_result = vm
        .eval(
            r#"
(() => {
  const parent = globalThis.__denseNestedMutationParent;
  const rules = globalThis.__denseNestedMutationRules;
  for (let index = 0; index < 1000; index++) {
    parent.insertRule(`.rule-${index} { --index: ${index}; }`, rules.length);
  }
  return rules.length;
})()
"#,
        )
        .expect("dense native grouping-rule insertion should evaluate");

    assert_eq!(mutation_result, "1000");
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::detached_rule_mutation_count_for_test(),
        0
    );
    let metrics = crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test();
    assert_eq!(metrics.native_top_level_mutations, 0);
    assert_eq!(metrics.native_nested_mutations, 1000);
    assert_eq!(metrics.native_keyframe_mutations, 0);
    assert_eq!(
        metrics.recursive_rule_snapshots, 0,
        "nested insertRule must seed read style wrappers without recursive projection"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const parent = globalThis.__denseNestedMutationParent;
  const rules = globalThis.__denseNestedMutationRules;
  const parentText = parent.cssText;
  const parentTextAgain = parent.cssText;
  return [
    rules.length,
    rules[0].cssText,
    rules[999].cssText,
    parentText.includes('.rule-0 { --index: 0; }'),
    parentText.includes('.rule-999 { --index: 999; }'),
    parentTextAgain === parentText,
  ].join('|');
})()
"#,
        )
        .expect("dense native grouping-rule insertion should evaluate");

    assert_eq!(
        result,
        "1000|.rule-0 { --index: 0; }|.rule-999 { --index: 999; }|true|true|true"
    );
}
#[test]
fn css_keyframes_rule_dense_append_avoids_full_stylesheet_projection() {
    let mut vm = new_storage_test_vm("https://css-keyframes-rule-native-density.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes dense { }');
  globalThis.__denseKeyframeMutationSheet = sheet;
  globalThis.__denseKeyframeMutationParent = sheet.cssRules[0];
  globalThis.__denseKeyframeMutationRules = sheet.cssRules[0].cssRules;
})()
"#,
    )
    .expect("dense native keyframes fixture should initialize");

    crate::context_bootstrap::css_stylesheet_runtime::reset_detached_rule_mutation_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let mutation_result = vm
        .eval(
            r#"
(() => {
  const parent = globalThis.__denseKeyframeMutationParent;
  const rules = globalThis.__denseKeyframeMutationRules;
  for (let index = 0; index < 1000; index++) {
    parent.appendRule(`${index / 10}% { --index: ${index}; }`);
  }
  return rules.length;
})()
"#,
        )
        .expect("dense native keyframe insertion should evaluate");

    assert_eq!(mutation_result, "1000");
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::detached_rule_mutation_count_for_test(),
        0
    );
    let metrics = crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test();
    assert_eq!(metrics.native_top_level_mutations, 0);
    assert_eq!(metrics.native_nested_mutations, 0);
    assert_eq!(metrics.native_keyframe_mutations, 1000);
    assert_eq!(
        metrics.recursive_rule_snapshots, 0,
        "appendRule must seed keyframe wrappers without full rule projection"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const parent = globalThis.__denseKeyframeMutationParent;
  const rules = globalThis.__denseKeyframeMutationRules;
  const parentText = parent.cssText;
  const parentTextAgain = parent.cssText;
  return [
    rules.length,
    rules[0].cssText,
    rules[999].cssText,
    parentText.includes('0% { --index: 0; }'),
    parentText.includes('99.9% { --index: 999; }'),
    parentTextAgain === parentText,
  ].join('|');
})()
"#,
        )
        .expect("dense native keyframe insertion should evaluate");

    assert_eq!(
        result,
        "1000|0% { --index: 0; }|99.9% { --index: 999; }|true|true|true"
    );
}
#[test]
fn css_style_rule_dense_value_mutation_stays_on_the_native_single_rule_path() {
    let mut vm = new_storage_test_vm("https://css-style-rule-native-value-density.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.subject { color: red; } .sibling { color: blue; }');
  globalThis.__denseValueMutationSheet = sheet;
  globalThis.__denseValueMutationRule = sheet.cssRules[0];
  globalThis.__denseValueMutationSibling = sheet.cssRules[1];
})()
"#,
    )
    .expect("dense native rule-value fixture should initialize");

    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__denseValueMutationSheet;
  const rule = globalThis.__denseValueMutationRule;
  const sibling = globalThis.__denseValueMutationSibling;
  for (let index = 0; index < 1000; index++) {
    rule.style.setProperty('--iteration', String(index));
  }
  return [
    sheet.cssRules[0] === rule,
    sheet.cssRules[1] === sibling,
    rule.style.getPropertyValue('--iteration'),
    rule.cssText,
    sibling.cssText,
  ].join('|');
})()
"#,
        )
        .expect("dense native rule-value mutation should evaluate");

    assert_eq!(
        result,
        "true|true|999|.subject { color: red; --iteration: 999; }|.sibling { color: blue; }"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0
    );
    let metrics = crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test();
    assert_eq!(metrics.native_rule_value_mutations, 1000);
    assert_eq!(metrics.recursive_rule_snapshots, 0);
}
#[test]
fn css_grouping_rule_replacement_keeps_new_children_sparse() {
    let mut vm = new_storage_test_vm("https://css-grouping-replacement-sparse.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { .old { color: red; } .unmaterialized { color: blue; } }');
  const media = sheet.cssRules[0];
  const rules = media.cssRules;
  globalThis.__sparseReplacementMedia = media;
  globalThis.__sparseReplacementRules = rules;
  globalThis.__sparseReplacementOld = rules[0];
})()
"#,
    )
    .expect("sparse replacement fixture should initialize");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const media = globalThis.__sparseReplacementMedia;
  const rules = globalThis.__sparseReplacementRules;
  const old = globalThis.__sparseReplacementOld;
  const children = Array.from(
    { length: 1000 },
    (_, index) => `.new-${index} { --index: ${index}; }`
  ).join('\n');
  media.cssText = `@media print { ${children} }`;
  return [
    media.cssRules === rules,
    rules.length,
    old.parentRule === null,
    old.parentStyleSheet === null,
    old.cssText,
  ].join('|');
})()
"#,
        )
        .expect("grouping replacement should reset its existing child list");

    assert_eq!(result, "true|1000|true|true|.old { color: red; }");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        0,
        "replacement must not materialize any rule from the new subtree"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test()
            .recursive_rule_snapshots,
        1,
        "replacement may snapshot only the retained old wrapper, not the new native subtree"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const rules = globalThis.__sparseReplacementRules;
  const first = rules[0];
  const middle = rules[500];
  const last = rules[999];
  return [
    first.cssText,
    middle.cssText,
    last.cssText,
    rules[0] === first,
    rules[500] === middle,
  ].join('|');
})()
"#,
        )
        .expect("replacement children should materialize on indexed access");

    assert_eq!(
        result,
        ".new-0 { --index: 0; }|.new-500 { --index: 500; }|.new-999 { --index: 999; }|true|true"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        3
    );
}
