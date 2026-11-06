use super::*;
use moli_page_types::LayoutPolicy;

fn realtime_vm(markup: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_parsed_test_vm("https://realtime-cssom.test/", markup);
    vm.set_layout_policy(LayoutPolicy::Realtime);
    vm
}

#[test]
fn realtime_cssom_retained_declaration_reads_current_used_sizes() {
    let mut vm = realtime_vm(
        "<!doctype html><div id=container style='width:200px'><div id=target style='width:50%;height:10px'></div></div>",
    );
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "const style=getComputedStyle(target);const values=[style.width];\
             container.style.width='400px';values.push(style.width,style.getPropertyValue('width'),style.inlineSize);\
             target.style.width='75%';values.push(style.getPropertyValue('width'));JSON.stringify(values)",
        )
        .unwrap(),
        r#"["100px","200px","200px","200px","300px"]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 3);
}

#[test]
fn realtime_cssom_pure_root_style_and_inline_reads_do_not_build_layout() {
    let mut vm =
        realtime_vm("<!doctype html><div id=target style='width:100px;color:rgb(1,2,3)'></div>");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "const style=getComputedStyle(target);target.style.width='200px';\
             JSON.stringify([style.color,style.getPropertyValue('color'),target.style.width])",
        )
        .unwrap(),
        r#"["rgb(1, 2, 3)","rgb(1, 2, 3)","200px"]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    assert_eq!(vm.eval("getComputedStyle(target).width").unwrap(), "200px");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_cssom_child_color_observes_current_parent_viewport() {
    let mut vm = realtime_vm(
        "<!doctype html><iframe id=frame style='width:200px;height:100px;border:0'></iframe>",
    );
    vm.eval(
        "window.child=frame.contentWindow;child.document.open();\
         child.document.write('<!doctype html><style>#target{color:rgb(1,2,3)}@media(min-width:250px){#target{color:rgb(4,5,6)}}</style><div id=target></div>');\
         child.document.close();window.style=child.getComputedStyle(child.document.getElementById('target'))",
    )
    .unwrap();
    assert_eq!(vm.eval("style.color").unwrap(), "rgb(1, 2, 3)");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval("frame.style.width='320px';style.getPropertyValue('color')")
            .unwrap(),
        "rgb(4, 5, 6)"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(vm.eval("style.color").unwrap(), "rgb(4, 5, 6)");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_cssom_grid_tracks_share_current_geometry() {
    let mut vm = realtime_vm(
        "<!doctype html><div id=target style='display:grid;width:200px;grid-template-columns:1fr 1fr'><div>A</div><div>B</div></div>",
    );
    assert_eq!(
        vm.eval("getComputedStyle(target).gridTemplateColumns")
            .unwrap(),
        "100px 100px"
    );
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "target.style.width='400px';JSON.stringify([getComputedStyle(target).getPropertyValue('grid-template-columns'),target.offsetWidth])",
        )
        .unwrap(),
        r#"["200px 200px",400]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_cssom_protocol_batch_refreshes_once_before_observation() {
    let mut vm = realtime_vm(
        "<!doctype html><div id=container style='width:200px'>\
         <div id=first style='width:50%;color:rgb(1,2,3)'></div>\
         <div id=second style='width:25%;color:rgb(1,2,3)'></div></div>",
    );
    let handles = {
        let host = vm._context_host.borrow();
        ["first", "second"].map(|id| host.dom_host().element_handle_by_id(id).unwrap())
    };
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.computed_style_property_values_for_document_snapshot(handles, &["color".into()])
            .unwrap(),
        vec![vec!["rgb(1, 2, 3)"], vec!["rgb(1, 2, 3)"]]
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    let properties = ["width", "inline-size"].map(String::from);
    assert_eq!(
        vm.computed_style_property_values_for_document_snapshot(handles, &properties)
            .unwrap(),
        vec![vec!["100px", "100px"], vec!["50px", "50px"]]
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    vm.eval("container.style.width='400px'").unwrap();
    for _ in 0..2 {
        assert_eq!(
            vm.computed_style_property_values_for_document_snapshot(handles, &properties)
                .unwrap(),
            vec![vec!["200px", "200px"], vec!["100px", "100px"]]
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
    }
}

#[test]
fn realtime_cssom_inspector_enumeration_refreshes_used_values() {
    let mut vm = realtime_vm("<!doctype html><div id=target style='width:100px'></div>");
    let target = vm
        ._context_host
        .borrow()
        .dom_host()
        .element_handle_by_id("target")
        .unwrap();
    let before = vm.layout_pass_observability_for_test().1;
    for (width, builds, change_width) in [(100, 1, false), (240, 2, true), (240, 2, false)] {
        if change_width {
            vm.eval("target.style.width='240px'").unwrap();
        }
        let properties = vm
            .computed_style_properties_for_inspector_handle(target)
            .unwrap()
            .unwrap()
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(properties["width"], format!("{width}px"));
        assert_eq!(vm.layout_pass_observability_for_test().1, before + builds);
    }
}

const CONTAINER_DEPENDENT_DURATION: &str = "calc(10s + (sign(2cqw - 10px) * 5s))";

fn container_duration_vm(
    animation_duration: &str,
    transition_duration: &str,
) -> StandaloneScriptVmHarness {
    let markup = format!(
        r#"<!doctype html><style>
        #container {{ container-type:inline-size; width:100px; height:20px }}
        </style><div id=container>
        <div id=target style="animation-duration:{animation_duration};transition-duration:{transition_duration};color:rgb(1,2,3)"></div>
        <div id=second style="animation-duration:{animation_duration};transition-duration:{transition_duration};color:rgb(1,2,3)"></div>
        </div>"#,
    );
    let mut vm = realtime_vm(&markup);
    // Establish the previous container geometry explicitly. Every tested
    // write/read below must refresh through CSSOM, without a geometry getter.
    assert_eq!(
        vm.eval(
            "window.containerRule=document.styleSheets[0].cssRules[0];\
             window.heldDurationStyle=getComputedStyle(document.getElementById('target'));\
             document.getElementById('container').offsetWidth",
        )
        .unwrap(),
        "100"
    );
    vm
}

#[test]
fn realtime_cssom_retained_duration_declaration_refreshes_container_dependencies() {
    for (js_property, css_property) in [
        ("animationDuration", "animation-duration"),
        ("transitionDuration", "transition-duration"),
    ] {
        for method_first in [false, true] {
            let mut vm =
                container_duration_vm(CONTAINER_DEPENDENT_DURATION, CONTAINER_DEPENDENT_DURATION);
            let getter = format!("heldDurationStyle.{js_property}");
            let method = format!("heldDurationStyle.getPropertyValue('{css_property}')");
            let (first, second) = if method_first {
                (&method, &getter)
            } else {
                (&getter, &method)
            };
            assert_eq!(vm.eval(first).unwrap(), "5s");
            let before = vm.layout_pass_observability_for_test().1;
            let reads = format!("JSON.stringify([{first},{second}])");
            assert_eq!(
                vm.eval(&format!("containerRule.style.width='1000px';{reads}"))
                    .expect("public duration read must refresh without reentering layout"),
                r#"["15s","15s"]"#,
                "{css_property}, method_first={method_first}"
            );
            assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
            assert_eq!(vm.eval(&reads).unwrap(), r#"["15s","15s"]"#);
            assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
        }
    }
}

#[test]
fn realtime_cssom_duration_shorthands_refresh_before_serializing_longhands() {
    for property in ["animation", "transition"] {
        let mut vm =
            container_duration_vm(CONTAINER_DEPENDENT_DURATION, CONTAINER_DEPENDENT_DURATION);
        assert_eq!(
            vm.eval(&format!("heldDurationStyle.{property}")).unwrap(),
            "5s"
        );
        let before = vm.layout_pass_observability_for_test().1;
        assert_eq!(
            vm.eval(&format!(
                "containerRule.style.width='1000px';heldDurationStyle.{property}"
            ))
            .unwrap(),
            "15s"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
        // getPropertyValue must also work as the first read after a different
        // mutation; the preceding getter cannot make this assertion pass.
        assert_eq!(
            vm.eval(&format!(
                "containerRule.style.width='100px';heldDurationStyle.getPropertyValue('{property}')"
            ))
            .unwrap(),
            "5s"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
        assert_eq!(
            vm.eval(&format!(
                "JSON.stringify([heldDurationStyle.{property},heldDurationStyle.getPropertyValue('{property}')])"
            ))
            .unwrap(),
            r#"["5s","5s"]"#
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
    }
}

#[test]
fn realtime_cssom_duration_protocol_batch_refreshes_once_before_style_observation() {
    let mut vm = container_duration_vm(CONTAINER_DEPENDENT_DURATION, CONTAINER_DEPENDENT_DURATION);
    let handles = {
        let host = vm._context_host.borrow();
        ["target", "second"].map(|id| host.dom_host().element_handle_by_id(id).unwrap())
    };
    // No used-size property is requested: these durations alone must establish
    // current container geometry before the batch owns its style observation.
    let properties = ["animation-duration", "transition-duration"].map(String::from);
    assert_eq!(
        vm.computed_style_property_values_for_document_snapshot(handles, &properties)
            .unwrap(),
        vec![vec!["5s", "5s"], vec!["5s", "5s"]]
    );
    let before = vm.layout_pass_observability_for_test().1;
    vm.eval("containerRule.style.width='1000px'").unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    for _ in 0..2 {
        assert_eq!(
            vm.computed_style_property_values_for_document_snapshot(handles, &properties)
                .expect("batch freshness must finish before its style borrow"),
            vec![vec!["15s", "15s"], vec!["15s", "15s"]]
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    }
}

#[test]
fn realtime_cssom_constant_durations_and_color_do_not_refresh_dirty_container_geometry() {
    let mut vm = container_duration_vm("2s", "3s");
    let handles = {
        let host = vm._context_host.borrow();
        ["target", "second"].map(|id| host.dom_host().element_handle_by_id(id).unwrap())
    };
    let before = vm.layout_pass_observability_for_test().1;
    let reads = "JSON.stringify([heldDurationStyle.animationDuration,\
        heldDurationStyle.getPropertyValue('animation-duration'),\
        heldDurationStyle.transitionDuration,\
        heldDurationStyle.getPropertyValue('transition-duration'),\
        heldDurationStyle.color,heldDurationStyle.getPropertyValue('color')])";
    assert_eq!(
        vm.eval(&format!("containerRule.style.width='1000px';{reads}"))
            .unwrap(),
        r#"["2s","2s","3s","3s","rgb(1, 2, 3)","rgb(1, 2, 3)"]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    let properties = ["animation-duration", "transition-duration", "color"].map(String::from);
    for _ in 0..2 {
        assert_eq!(
            vm.computed_style_property_values_for_document_snapshot(handles, &properties)
                .unwrap(),
            vec![
                vec!["2s", "3s", "rgb(1, 2, 3)"],
                vec!["2s", "3s", "rgb(1, 2, 3)"]
            ]
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before);
    }
    assert_eq!(
        vm.eval(reads).unwrap(),
        r#"["2s","2s","3s","3s","rgb(1, 2, 3)","rgb(1, 2, 3)"]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    // Confirm that the write was real and the geometry stayed dirty throughout
    // the pure-style reads, rather than accidentally exercising a clean page.
    assert_eq!(vm.eval("container.offsetWidth").unwrap(), "1000");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_cssom_container_numeric_var_fallbacks_refresh_but_constant_variables_do_not() {
    // Inline container sizes exercise the engine's existing Stylo query_size
    // input for var() substitution. Rule-defined used sizes are independently
    // covered by the direct supplemental-value regression above.
    for invalid_definition in ["", "--measure:var(--measure);"] {
        let expression = "var(--measure, calc(10s + (sign(2cqw - 10px) * 5s)))";
        let mut vm = realtime_vm(&format!(
            r#"<!doctype html><div id=container style="container-type:inline-size;width:100px;height:20px">
                <div id=target style="{invalid_definition}animation-duration:{expression};transition-duration:{expression}"></div>
            </div>"#,
        ));
        assert_eq!(
            vm.eval("window.variableStyle=getComputedStyle(target);container.offsetWidth")
                .unwrap(),
            "100"
        );
        assert_eq!(vm.eval(
            "JSON.stringify([variableStyle.getPropertyValue('--measure'),variableStyle.animationDuration,variableStyle.transitionDuration])"
        ).unwrap(), r#"["","5s","5s"]"#);
        let before = vm.layout_pass_observability_for_test().1;
        assert_eq!(vm.eval(
            "container.style.width='1000px';JSON.stringify([variableStyle.animationDuration,variableStyle.getPropertyValue('transition-duration')])"
        ).unwrap(), r#"["15s","15s"]"#);
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
        assert_eq!(
            vm.eval(
                "JSON.stringify([variableStyle.animationDuration,variableStyle.transitionDuration])"
            )
            .unwrap(),
            r#"["15s","15s"]"#
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    }

    let mut vm = realtime_vm(
        r#"<!doctype html><div id=container style="container-type:inline-size;width:100px;height:20px;--measure:2s">
            <div id=target style="animation-duration:var(--measure,calc(10s + (sign(2cqw - 10px) * 5s)));transition-duration:var(--measure,calc(10s + (sign(2cqw - 10px) * 5s)))"></div>
        </div>"#,
    );
    assert_eq!(
        vm.eval("window.variableStyle=getComputedStyle(target);container.offsetWidth")
            .unwrap(),
        "100"
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify([variableStyle.animationDuration,variableStyle.transitionDuration])"
        )
        .unwrap(),
        r#"["2s","2s"]"#
    );
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(vm.eval(
        "container.style.width='1000px';JSON.stringify([variableStyle.animationDuration,variableStyle.getPropertyValue('transition-duration')])"
    ).unwrap(), r#"["2s","2s"]"#);
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    assert_eq!(
        vm.eval(
            "JSON.stringify([variableStyle.animationDuration,variableStyle.transitionDuration])"
        )
        .unwrap(),
        r#"["2s","2s"]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
}

#[test]
fn realtime_cssom_inherited_zoom_refreshes_the_source_container_dependency() {
    // The existing computed_style_resolves_zoom regression already supports
    // zoom:inherit and this numeric cq expression. Keep the source as a child
    // of the size container, so its own query never depends on itself.
    let mut vm = realtime_vm(
        r#"<!doctype html><style>#container {container-type:inline-size;width:100px;height:20px}</style>
        <div id=container><div id=zoomSource style="zoom:calc(100% + (sign(2cqw - 10px) * 50%))">
        <div id=target style="zoom:inherit"></div></div></div>"#,
    );
    assert_eq!(vm.eval(
        "window.zoomRule=document.styleSheets[0].cssRules[0];window.zoomStyle=getComputedStyle(target);container.offsetWidth"
    ).unwrap(), "100");
    assert_eq!(vm.eval("zoomStyle.zoom").unwrap(), "0.5");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(vm.eval(
        "zoomRule.style.width='1000px';JSON.stringify([zoomStyle.zoom,zoomStyle.getPropertyValue('zoom')])"
    ).unwrap(), r#"["1.5","1.5"]"#);
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(vm.eval("zoomStyle.zoom").unwrap(), "1.5");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_cssom_existing_custom_function_projection_refreshes_container_width() {
    // Reuse the existing no-argument renderer @function projection; this does
    // not assert new stylesheet container-query or arbitrary function support.
    let mut vm = realtime_vm(
        r#"<!doctype html><style>
        #container {container:--measure / size;width:100px;height:20px}
        @function --measured() {
            @container --measure (width = 100px) {result:100px;}
            @container --measure (width = 1000px) {result:1000px;}
        }
        #target {--actual:--measured();--literal:1cqw}
        </style><div id=container><div id=target></div></div>"#,
    );
    assert_eq!(vm.eval(
        "window.functionRule=document.styleSheets[0].cssRules[0];window.functionStyle=getComputedStyle(target);container.offsetWidth"
    ).unwrap(), "100");
    assert_eq!(
        vm.eval("functionStyle.getPropertyValue('--actual')")
            .unwrap(),
        "100px"
    );
    let before = vm.layout_pass_observability_for_test().1;
    // Literal custom-property tokens alone do not measure the container.
    assert_eq!(
        vm.eval("functionRule.style.width='1000px';functionStyle.getPropertyValue('--literal')")
            .unwrap(),
        "1cqw"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    assert_eq!(
        vm.eval("functionStyle.getPropertyValue('--actual')")
            .unwrap(),
        "1000px"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(
        vm.eval("functionStyle.getPropertyValue('--actual')")
            .unwrap(),
        "1000px"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}
