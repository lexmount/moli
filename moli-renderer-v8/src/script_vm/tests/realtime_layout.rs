use super::*;
use moli_page_types::LayoutPolicy;

fn realtime_vm(markup: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_parsed_test_vm("https://realtime-layout.test/", markup);
    vm.set_layout_policy(LayoutPolicy::Realtime);
    vm
}

#[test]
fn realtime_layout_writes_and_reads_in_one_call_share_only_current_inputs() {
    let mut vm = realtime_vm(
        "<!doctype html><div id=target style='width:100px;height:10px;padding:0;border:0'></div>",
    );
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "const widths=[target.offsetWidth];target.style.width='200px';\
             widths.push(target.offsetWidth,target.offsetWidth);\
             target.style.width='300px';widths.push(target.offsetWidth);JSON.stringify(widths)",
        )
        .unwrap(),
        "[100,200,200,300]"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 3);
    vm.eval("getComputedStyle(target).color;target.getBoundingClientRect();target.offsetWidth")
        .unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 3);
    assert_eq!(
        vm.layout_pass_observability_for_test()
            .3
            .unwrap()
            .paint_operation_count,
        0
    );
}

#[test]
fn realtime_layout_observes_cssom_sources_without_dom_attribute_mutations() {
    let mut vm = realtime_vm(
        "<!doctype html><style>#target{width:100px;height:10px}</style><div id=target></div>",
    );
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "document.styleSheets[0].cssRules[0].style.width='240px';String(target.offsetWidth)",
        )
        .unwrap(),
        "240"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(
        vm.eval(
            "const sheet=new CSSStyleSheet();sheet.replaceSync('#target{width:360px}');\
             document.adoptedStyleSheets=[sheet];String(target.offsetWidth)",
        )
        .unwrap(),
        "360"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "360");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
}

#[test]
fn realtime_layout_refresh_does_not_call_page_callbacks() {
    let mut vm = realtime_vm("<!doctype html><div id=target style='width:100px'></div>");
    assert_eq!(
        vm.eval(
            "window.callbacks=[];\
             new MutationObserver(()=>callbacks.push('mutation')).observe(target,{attributes:true});\
             requestAnimationFrame(()=>callbacks.push('frame'));\
             target.offsetWidth;target.style.width='200px';\
             JSON.stringify([target.offsetWidth,callbacks.slice()])",
        )
        .unwrap(),
        "[200,[]]"
    );
}

#[test]
fn realtime_layout_child_reads_refresh_parent_viewport_without_replacing_root() {
    let mut vm = realtime_vm(
        "<!doctype html><iframe id=frame style='width:200px;height:100px;border:0'></iframe>",
    );
    vm.eval(
        "window.child=frame.contentWindow;child.document.open();\
         child.document.write('<!doctype html><body style=margin:0><div id=target style=width:100%;height:10px></div>');\
         child.document.close();window.childTarget=child.document.getElementById('target')",
    )
    .unwrap();
    assert_eq!(vm.eval("String(childTarget.offsetWidth)").unwrap(), "200");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "frame.style.width='320px';JSON.stringify([childTarget.offsetWidth,frame.offsetWidth,childTarget.offsetWidth])",
        )
        .unwrap(),
        "[320,320,320]"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_layout_does_not_keep_geometry_from_a_replaced_document() {
    let mut vm = realtime_vm("<!doctype html><div id=target style='width:100px'></div>");
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "document.open();document.write('<!doctype html><div id=newTarget style=width:270px></div>');\
             document.close();String(newTarget.offsetWidth)",
        )
        .unwrap(),
        "270"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_layout_is_current_inside_parser_and_custom_element_reactions() {
    let mut vm = realtime_vm("<!doctype html><body></body>");
    // The same fixture runs in Chromium and through the release CDP entry.
    // Parser writes must invalidate style/geometry before connected reactions;
    // subsequent writes and attribute reactions must be visible in this turn,
    // including through an already held CSSStyleDeclaration and in a child.
    let fixture = include_str!("../../../tests/fixtures/realtime-parser-reactions.js");
    let result = vm.eval(&format!("JSON.stringify({fixture})")).unwrap();
    let expected = serde_json::json!([
        ["initial", 100, "100px"],
        ["connected", 210, "210px", 210, 75],
        ["write", 320, "320px"],
        ["attribute", 430, "430px"],
        ["after", 430, "430px"]
    ]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([expected, expected])
    );
    assert_eq!(
        vm.layout_pass_observability_for_test()
            .3
            .unwrap()
            .paint_operation_count,
        0
    );
}

#[test]
fn realtime_layout_native_state_and_scroll_commit_before_geometry_reads() {
    let mut vm = realtime_vm(
        "<!doctype html><style>#scroller{width:100px}#flag:checked~#scroller{width:200px}</style>\
         <input id=flag type=checkbox><div id=scroller style='height:50px;overflow:auto'>\
         <div id=target style='width:20px;height:500px'></div></div>",
    );
    assert_eq!(vm.eval("String(scroller.offsetWidth)").unwrap(), "100");
    assert_eq!(
        vm.eval(
            "flag.checked=true;\
             const before=target.getBoundingClientRect().top;scroller.scrollTop=30;\
             JSON.stringify([scroller.offsetWidth,before-target.getBoundingClientRect().top])",
        )
        .unwrap(),
        "[200,30]"
    );
}

#[test]
fn legacy_layout_modes_do_not_allocate_realtime_state_or_refresh_after_writes() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let mut vm = new_parsed_test_vm(
            "https://legacy-layout.test/",
            "<!doctype html><div id=target style='width:100px'></div><input id=field type=button value=A>",
        );
        vm.set_layout_policy(policy);
        assert!(
            !vm._context_host
                .borrow()
                .realtime_layout_allocated_for_test()
        );
        vm.eval("target.offsetWidth").unwrap();
        let before = vm.layout_pass_observability_for_test().1;
        let width = vm
            .eval("target.style.width='200px';field.value='ABCDEFGHIJKLMNOPQRST';String(target.offsetWidth)")
            .unwrap();
        assert_eq!(vm.eval("field.value").unwrap(), "ABCDEFGHIJKLMNOPQRST");
        assert_eq!(vm.layout_pass_observability_for_test().1, before);
        if policy == LayoutPolicy::OnDemand {
            assert_eq!(width, "100");
        }
        assert!(
            !vm._context_host
                .borrow()
                .realtime_layout_allocated_for_test()
        );
    }
}

#[test]
fn select_value_refreshes_selected_peer_and_has_styles_in_every_layout_policy() {
    for policy in [
        LayoutPolicy::Mock,
        LayoutPolicy::OnDemand,
        LayoutPolicy::Realtime,
    ] {
        for (case, change, same_before, same_after) in [
            (
                "selectedIndex",
                "selectCtl.selectedIndex=1",
                "selectCtl.selectedIndex=0",
                "selectCtl.selectedIndex=1",
            ),
            (
                "value",
                "selectCtl.value='b'",
                "selectCtl.value='a'",
                "selectCtl.value='b'",
            ),
        ] {
            let mut vm = new_parsed_test_vm(
                "https://select-value-style.test/",
                r#"<!doctype html><style>
                    option { color:rgb(1,2,3) }
                    option:checked { color:rgb(4,5,6) }
                    #target { color:rgb(7,8,9) }
                    body:has(#optionB:checked) #target { color:rgb(10,11,12) }
                </style><select id=selectCtl>
                    <option id=optionA value=a>Alpha</option>
                    <option id=optionB value=b>Beta</option>
                </select><div id=target></div>"#,
            );
            vm.set_layout_policy(policy);
            assert_eq!(
                vm.eval(
                    r#"
                    // Set both dirty flags before checking identical-value writes.
                    selectCtl.selectedIndex=0;
                    window.aStyle=getComputedStyle(optionA);
                    window.bStyle=getComputedStyle(optionB);
                    window.targetStyle=getComputedStyle(target);
                    window.styleColors=()=>[aStyle.color,bStyle.color,targetStyle.color];
                    JSON.stringify(styleColors())
                "#
                )
                .unwrap(),
                r#"["rgb(4, 5, 6)","rgb(1, 2, 3)","rgb(7, 8, 9)"]"#,
                "initial held declarations: {policy:?}, {case}"
            );
            let builds = vm.layout_pass_observability_for_test().1;
            let version = vm._context_host.borrow().dom_host().dom_version();
            vm.eval(same_before).unwrap();
            assert_eq!(vm._context_host.borrow().dom_host().dom_version(), version);
            assert_eq!(vm.layout_pass_observability_for_test().1, builds);

            vm.eval(change).unwrap();
            assert_eq!(
                vm.eval("JSON.stringify([selectCtl.value,optionA.selected,optionB.selected])")
                    .unwrap(),
                r#"["b",false,true]"#,
                "native selection must actually change: {policy:?}, {case}"
            );
            assert!(vm._context_host.borrow().dom_host().dom_version() > version);
            assert_eq!(
                vm.eval("JSON.stringify(styleColors())").unwrap(),
                r#"["rgb(1, 2, 3)","rgb(4, 5, 6)","rgb(10, 11, 12)"]"#,
                "selected peer and :has styles must be current: {policy:?}, {case}"
            );
            assert_eq!(
                vm.layout_pass_observability_for_test().1,
                builds,
                "pure color reads must not repair this through a layout pass"
            );
            let changed_version = vm._context_host.borrow().dom_host().dom_version();
            vm.eval(same_after).unwrap();
            assert_eq!(
                vm._context_host.borrow().dom_host().dom_version(),
                changed_version
            );
            assert_eq!(vm.layout_pass_observability_for_test().1, builds);
            assert_eq!(
                vm.eval("JSON.stringify(styleColors())").unwrap(),
                r#"["rgb(1, 2, 3)","rgb(4, 5, 6)","rgb(10, 11, 12)"]"#
            );
            if !policy.is_realtime() {
                assert!(
                    !vm._context_host
                        .borrow()
                        .realtime_layout_allocated_for_test()
                );
            }
        }
    }
}

#[test]
fn selection_changes_refresh_required_select_and_container_validity_styles() {
    const INVALID_COLORS: &str = r#"["rgb(1, 2, 3)","rgb(1, 2, 3)","rgb(1, 2, 3)"]"#;
    const VALID_COLORS: &str = r#"["rgb(4, 5, 6)","rgb(4, 5, 6)","rgb(4, 5, 6)"]"#;
    let mut observed = Vec::new();
    for policy in [
        LayoutPolicy::Mock,
        LayoutPolicy::OnDemand,
        LayoutPolicy::Realtime,
    ] {
        for (case, change) in [
            ("selectedIndex", "validitySelect.selectedIndex=1"),
            ("value", "validitySelect.value='b'"),
            ("option.selected", "validityChoice.selected=true"),
        ] {
            let mut vm = new_parsed_test_vm(
                "https://select-validity-style.test/",
                r#"<!doctype html><style>
                  #validitySelect, #validityForm, #validityFieldset { color:rgb(1,2,3) }
                  #validitySelect:valid, #validityForm:valid, #validityFieldset:valid {
                    color:rgb(4,5,6)
                  }
                </style><form id=validityForm><fieldset id=validityFieldset>
                  <select id=validitySelect required>
                    <option id=validityEmpty value="" selected>Choose</option>
                    <option id=validityChoice value=b>Beta</option>
                  </select>
                </fieldset></form>"#,
            );
            vm.set_layout_policy(policy);
            assert_eq!(
                vm.eval(
                    r#"
                    // Prime native dirty flags without relying on implicit
                    // first-option selectedness or changing the chosen option.
                    validitySelect.selectedIndex=0;
                    window.validityTargets=[validitySelect,validityForm,validityFieldset];
                    window.validityStyles=validityTargets.map(el=>getComputedStyle(el));
                    window.validityColors=()=>JSON.stringify(validityStyles.map(style=>style.color));
                    window.validityState=()=>JSON.stringify([
                      validitySelect.value,
                      validitySelect.validity.valueMissing,
                      validitySelect.validity.valid,
                      // Compound selectors bypass matches(':valid')'s native
                      // shortcut and exercise the Stylo selector route.
                      validityTargets.map(el=>el.matches('#'+el.id+':valid')),
                      validitySelect.form===validityForm
                    ]);
                    validityState()
                    "#,
                )
                .unwrap(),
                r#"["",true,false,[false,false,false],true]"#,
                "live required-select and supported compound selectors: {policy:?}, {case}"
            );
            assert_eq!(vm.eval("validityColors()").unwrap(), INVALID_COLORS);

            // Existing attribute invalidation is an independent positive
            // control for the same held declarations and validity selectors.
            assert_eq!(
                vm.eval("validitySelect.required=false;validityColors()")
                    .unwrap(),
                VALID_COLORS,
                "attribute route must refresh the same cached styles: {policy:?}, {case}"
            );
            assert_eq!(
                vm.eval("validityState()").unwrap(),
                r#"["",false,true,[true,true,true],true]"#,
                "removing required changes the live constraints: {policy:?}, {case}"
            );
            assert_eq!(
                vm.eval("validitySelect.required=true;validityColors()")
                    .unwrap(),
                INVALID_COLORS
            );
            assert_eq!(
                vm.eval("validityState()").unwrap(),
                r#"["",true,false,[false,false,false],true]"#
            );

            let builds = vm.layout_pass_observability_for_test().1;
            // Observe held styles in the setter's JS call, before validity or
            // selector queries can refresh any state on behalf of this read.
            let colors = vm.eval(&format!("{change};validityColors()")).unwrap();
            assert_eq!(
                vm.eval("validityState()").unwrap(),
                r#"["b",false,true,[true,true,true],true]"#,
                "native validity and selector matching must agree: {policy:?}, {case}"
            );
            assert_eq!(
                vm.layout_pass_observability_for_test().1,
                builds,
                "pure color reads must not repair state with layout: {policy:?}, {case}"
            );

            // All dirty flags were primed above: writing the same selection is
            // a genuine no-op and must leave the observed style state stable.
            let version = vm._context_host.borrow().dom_host().dom_version();
            assert_eq!(
                vm.eval(&format!("{change};validityColors()")).unwrap(),
                colors
            );
            assert_eq!(vm._context_host.borrow().dom_host().dom_version(), version);
            assert_eq!(vm.layout_pass_observability_for_test().1, builds);
            if !policy.is_realtime() {
                assert!(
                    !vm._context_host
                        .borrow()
                        .realtime_layout_allocated_for_test()
                );
            }
            observed.push((format!("{policy:?}:{case}"), colors));
        }
    }
    // Keep the observations from all shared setters/modes before reporting
    // stale colors; selectedIndex is not assumed to be a correct baseline.
    assert!(
        observed.iter().all(|(_, colors)| colors == VALID_COLORS),
        "held colors ordered as [select, form, fieldset]: {observed:?}"
    );
}
