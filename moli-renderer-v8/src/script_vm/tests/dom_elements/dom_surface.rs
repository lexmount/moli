use super::*;

async fn expect_one_child_frame_task_source(
    vm: &mut ScriptVm,
    expected: impl Into<ChildFrameSemanticTurnKind>,
    context: &str,
) {
    let expected = expected.into();
    let source = vm.run_next_child_frame_semantic_turn_for_test().await;
    assert_eq!(source, Some(expected), "{context}");
}

/// Settle at most one realm-materialization prerequisite before observing the
/// requested child-family turn. This is test setup, not a one-turn executor.
async fn expect_child_frame_task_source_after_realm_prerequisite(
    vm: &mut ScriptVm,
    expected: impl Into<ChildFrameSemanticTurnKind>,
    context: &str,
) {
    let expected = expected.into();
    if expected != ChildFrameSemanticTurnKind::RealmMaterialization
        && vm.has_ready_child_frame_semantic_turn_for_test(
            ChildFrameSemanticTurnKind::RealmMaterialization,
        )
    {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::RealmMaterialization),
            "{context}: exact child realm prerequisite"
        );
    }
    expect_one_child_frame_task_source(vm, expected, context).await;
}

/// Observe one exact child semantic family through the production Page
/// selected-task dispatcher, allowing the single realm-materialization
/// prerequisite that can precede it.
async fn expect_page_child_frame_task_source_after_realm_prerequisite(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    expected: impl Into<ChildFrameSemanticTurnKind>,
    context: &str,
) {
    let expected = expected.into();
    if expected != ChildFrameSemanticTurnKind::RealmMaterialization
        && page
            .run_one_child_frame_task_executor_turn(
                ChildFrameSemanticTurnKind::RealmMaterialization,
                loader,
            )
            .await
            .expect("exact child realm prerequisite should run")
    {
        // Realm materialization is the only prerequisite this helper may
        // consume before the requested family.
    }
    assert!(
        page.run_one_child_frame_task_executor_turn(expected, loader)
            .await
            .expect("exact child semantic task should run"),
        "{context}"
    );
}

fn drain_image_load_event_bodies_for_test(vm: &mut ScriptVm) -> usize {
    let mut count = 0;
    while vm
        .apply_next_image_load_event_body_for_test()
        .expect("DOM-manipulation task should run")
    {
        count += 1;
    }
    count
}
mod extracted;

#[test]
fn inline_element_exposes_geometry_for_promoted_block_children() {
    for children in [
        "<b style='display:block;width:120px;height:20px'>X</b>",
        "<span style='display:flex;width:120px;height:20px'>X</span><b style='display:block;width:100px;height:20px'>Y</b>",
    ] {
        let mut vm = new_parsed_test_vm(
            "https://inline-block-fragments.test/",
            &format!("<!doctype html><html><body><a id='link'>{children}</a></body></html>"),
        );
        let result = vm
            .eval(
                "JSON.stringify({width:document.getElementById('link').getBoundingClientRect().width,rects:[...document.getElementById('link').getClientRects()].map(rect=>rect.width)})",
            )
            .expect("inline geometry should evaluate");
        let geometry: serde_json::Value = serde_json::from_str(&result).expect("geometry JSON");
        assert!(
            geometry["width"].as_f64().is_some_and(|width| width > 0.0),
            "a visible block child must contribute to its inline parent's bounding rect: {geometry}"
        );
        assert!(
            geometry["rects"].as_array().is_some_and(|rects| rects
                .iter()
                .any(|rect| rect.as_f64().is_some_and(|width| width > 0.0))),
            "a visible block child must contribute a nonempty client rect: {geometry}"
        );
    }
}

#[test]
fn offset_parent_observes_connected_insertion_after_paint_publication() {
    let mut vm = new_storage_test_vm("https://offset-parent-layout.test/");
    vm.eval(
        r#"
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.head) document.documentElement.appendChild(document.createElement('head'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        document.head.innerHTML = `<style>
          #menu { position:absolute; width:240px }
          #candidate { position:relative; display:flex; width:100%; height:32px }
        </style>`;
        document.body.innerHTML = '<div id="existing" style="width:40px;height:20px"></div>';
        document.getElementById('existing').getBoundingClientRect().width;
        globalThis.menu = document.createElement('div');
        menu.id = 'menu';
        globalThis.candidate = document.createElement('button');
        candidate.id = 'candidate';
        candidate.textContent = 'Abishek S abisubramanya27';
        menu.appendChild(candidate);
        'installed';
        "#,
    )
    .expect("fixture and initial layout should evaluate");

    let passes_after_initial_layout = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval("candidate.offsetParent === null && candidate.offsetWidth === 0")
            .unwrap(),
        "true",
        "geometry for a detached candidate must remain empty"
    );
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        passes_after_initial_layout,
        "building and querying a detached subtree must not invalidate live layout"
    );

    vm.eval("document.body.appendChild(menu); 'connected'")
        .expect("connected insertion should evaluate");
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        passes_after_initial_layout,
        "the connected insertion must mark layout dirty without rebuilding it eagerly"
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const parent = candidate.offsetParent;
              const width = candidate.offsetWidth;
              return JSON.stringify({ parentIsMenu: parent === menu, width });
            })()
            "#,
        )
        .expect("offsetParent-first geometry read should evaluate");
    assert_eq!(
        result, r#"{"parentIsMenu":false,"width":0}"#,
        "the inserted element is absent from the published layout"
    );
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        passes_after_initial_layout,
        "geometry reads must not publish a new layout"
    );

    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
        .expect("fresh paint layout should succeed")
        .expect("the fixture should have a layout root");
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        passes_after_initial_layout + 1
    );

    assert_eq!(
        vm.eval("candidate.offsetParent === menu && candidate.offsetWidth === 240")
            .unwrap(),
        "true"
    );
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        passes_after_initial_layout + 1,
        "geometry reads must reuse the published tree"
    );
}

#[test]
fn geometry_reuses_clean_layout_after_detached_subtree_mutations() {
    let mut vm = new_storage_test_vm("https://layout-detached.test/");
    vm.eval(
        r#"
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        document.body.innerHTML = '<div id="target" style="width:40px;height:20px"></div>';
        globalThis.target = document.getElementById('target');
        globalThis.detached = document.createElement('div');
        target.getBoundingClientRect().width;
    "#,
    )
    .expect("fixture should evaluate");
    let passes = vm.layout_pass_observability_for_test().1;
    for mutation in [
        "detached.textContent = 'not on the page'",
        "detached.appendChild(document.createElement('span'))",
        "detached.style.width = '300px'",
        "detached.setAttribute('data-wide', '')",
        "detached.firstChild.data = 'changed text'",
    ] {
        vm.eval(mutation)
            .expect("detached mutation should evaluate");
        assert_eq!(
            vm.eval("target.getBoundingClientRect().width").unwrap(),
            "40"
        );
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            passes,
            "detached mutation must not invalidate page layout: {mutation}"
        );
    }
}

#[test]
fn geometry_publishes_pseudo_class_inputs_at_paint_boundary() {
    let mut vm = new_storage_test_vm("https://layout-state.test/");
    vm.eval(r#"
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        document.body.innerHTML = '<style>input { width: 40px; height: 20px } input:focus { width: 80px } input:checked { width: 120px }</style><input id="target" type="checkbox">';
        globalThis.target = document.getElementById('target');
        target.getBoundingClientRect().width;
    "#).expect("fixture should evaluate");
    let mut passes = vm.layout_pass_observability_for_test().1;
    for (mutation, previous, expected) in [
        ("target.focus({preventScroll: true})", "40", "80"),
        ("target.checked = true", "80", "120"),
        ("target.checked = false", "120", "80"),
        ("target.blur()", "80", "40"),
    ] {
        vm.eval(mutation).expect("element state should change");
        assert_eq!(vm.layout_pass_observability_for_test().1, passes);
        assert_eq!(
            vm.eval("target.getBoundingClientRect().width").unwrap(),
            previous,
            "geometry must retain the previous published pseudo-class state: {mutation}"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, passes);
        vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
            .expect("fresh paint layout should succeed")
            .expect("the fixture should have a layout root");
        passes += 1;
        assert_eq!(vm.layout_pass_observability_for_test().1, passes);
        assert_eq!(
            vm.eval("target.getBoundingClientRect().width").unwrap(),
            expected
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, passes);
    }
}

#[test]
fn geometry_publishes_attribute_selectors_and_cssom_at_paint_boundary() {
    let mut vm = new_storage_test_vm("https://layout-inputs.test/");
    vm.eval(r#"
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        document.body.innerHTML = '<style id="rules">#target { width: 40px; height: 20px } #target[data-wide] { width: 80px }</style><div id="target"></div>';
        globalThis.target = document.getElementById('target');
        target.getBoundingClientRect().width;
    "#).expect("fixture should evaluate");
    let mut passes = vm.layout_pass_observability_for_test().1;
    for (mutation, previous, expected) in [
        ("target.setAttribute('data-wide', '')", "40", "80"),
        (
            "document.getElementById('rules').sheet.insertRule('#target[data-wide] { width: 120px }', 2)",
            "80",
            "120",
        ),
        ("target.removeAttribute('data-wide')", "120", "40"),
    ] {
        vm.eval(mutation).expect("layout input should change");
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            passes,
            "input changes must not eagerly lay out: {mutation}"
        );
        assert_eq!(
            vm.eval("target.getBoundingClientRect().width").unwrap(),
            previous,
            "geometry must retain the published style before capture: {mutation}"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, passes);
        vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
            .expect("fresh paint layout should succeed")
            .expect("the fixture should have a layout root");
        passes += 1;
        assert_eq!(vm.layout_pass_observability_for_test().1, passes);
        assert_eq!(
            vm.eval("target.getBoundingClientRect().width").unwrap(),
            expected
        );
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            passes,
            "clean geometry must be reusable across script turns"
        );
    }
}

#[test]
fn geometry_publishes_inserted_and_existing_nodes_at_paint_boundary() {
    let mut vm = new_storage_test_vm("https://layout-mutation.test/");

    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.body.innerHTML = '<main id="content"><div id="marker">Marker</div></main>';
          return document.getElementById("marker").getBoundingClientRect().top;
        })()
        "#,
    )
    .expect("initial layout read should evaluate");

    let before_top = vm
        .eval("document.getElementById('marker').getBoundingClientRect().top")
        .expect("initial marker geometry should evaluate")
        .parse::<f64>()
        .expect("initial marker top should be numeric");

    let passes_before = vm.layout_pass_observability_for_test().1;
    vm.eval(
        r#"document.getElementById("content").insertAdjacentHTML(
          "afterbegin",
          '<div style="height: 80px"></div><table><tr><td><button id="target">Filters</button></td></tr></table>'
        )"#,
    )
    .expect("connected insertion should evaluate");
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        passes_before,
        "a DOM mutation alone must not run layout"
    );
    assert!(
        vm.layout_snapshot_cache_observability_for_test()
            .3
            .is_some(),
        "mutation should retain the sampled frozen tree"
    );

    let stale = vm
        .eval(
            r#"
            (() => {
              const rect = document.getElementById("target").getBoundingClientRect();
              const marker = document.getElementById("marker").getBoundingClientRect();
              return JSON.stringify({ target: [rect.width, rect.height], markerTop: marker.top });
            })()
            "#,
        )
        .expect("geometry before layout publication should evaluate");

    let stale: serde_json::Value =
        serde_json::from_str(&stale).expect("geometry result should be JSON");
    assert_eq!(stale["target"], serde_json::json!([0, 0]));
    assert_eq!(stale["markerTop"].as_f64(), Some(before_top));
    assert_eq!(vm.layout_pass_observability_for_test().1, passes_before);

    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
        .expect("fresh paint layout should succeed")
        .expect("the fixture should have a layout root");
    let result = vm
        .eval(
            r#"(() => {
              const rect = document.getElementById("target").getBoundingClientRect();
              const marker = document.getElementById("marker").getBoundingClientRect();
              return JSON.stringify({ target: [rect.width, rect.height], markerTop: marker.top });
            })()"#,
        )
        .expect("published geometry should evaluate");

    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        passes_before + 1,
        "all geometry reads after capture should share one published pass"
    );

    let result: serde_json::Value =
        serde_json::from_str(&result).expect("geometry result should be JSON");
    assert!(
        result["target"][0]
            .as_f64()
            .is_some_and(|width| width > 0.0)
            && result["target"][1]
                .as_f64()
                .is_some_and(|height| height > 0.0),
        "inserted content should receive real geometry: {result}"
    );
    assert!(
        result["markerTop"]
            .as_f64()
            .is_some_and(|top| top > before_top),
        "existing geometry must not be reused after a preceding sibling is inserted: {result}"
    );
    vm.eval("const content = document.getElementById('content'); content.firstElementChild.remove(); content.firstElementChild.remove()")
        .expect("connected removals should evaluate");
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        passes_before + 1,
        "removal itself must not run layout"
    );
    assert_eq!(
        vm.eval("document.getElementById('marker').getBoundingClientRect().top")
            .unwrap()
            .parse::<f64>()
            .unwrap(),
        result["markerTop"].as_f64().unwrap(),
        "connected removal must leave the published geometry frozen"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, passes_before + 1);
    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
        .expect("fresh paint layout after removal should succeed")
        .expect("the fixture should have a layout root");
    assert_eq!(
        vm.eval("document.getElementById('marker').getBoundingClientRect().top")
            .unwrap()
            .parse::<f64>()
            .unwrap(),
        before_top,
        "a subsequent paint must publish geometry after connected removal"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, passes_before + 2);
}
