use super::*;
use crate::runtime::{
    RendererAutofillCreditCard, RendererAutofillTriggerOutcome, RendererAutofillTriggerRequest,
};
use moli_page_types::LayoutPolicy;

#[test]
fn select_value_refreshes_selected_peer_and_has_styles_in_every_layout_policy() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
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

            let colors = vm
                .eval(&format!("{change};JSON.stringify(styleColors())"))
                .unwrap();
            assert_eq!(
                vm.eval("JSON.stringify([selectCtl.value,optionA.selected,optionB.selected])")
                    .unwrap(),
                r#"["b",false,true]"#,
                "native selection must actually change: {policy:?}, {case}"
            );
            assert!(vm._context_host.borrow().dom_host().dom_version() > version);
            assert_eq!(
                colors, r#"["rgb(1, 2, 3)","rgb(4, 5, 6)","rgb(10, 11, 12)"]"#,
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
        }
    }
}

#[test]
fn selection_changes_refresh_required_select_and_container_validity_styles() {
    const INVALID_COLORS: &str = r#"["rgb(1, 2, 3)","rgb(1, 2, 3)","rgb(1, 2, 3)"]"#;
    const VALID_COLORS: &str = r#"["rgb(4, 5, 6)","rgb(4, 5, 6)","rgb(4, 5, 6)"]"#;
    let mut observed = Vec::new();
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
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

#[test]
fn select_autofill_refreshes_selected_peer_and_has_styles_before_callbacks() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let mut vm = new_parsed_test_vm(
            "https://select-autofill-state-style.test/",
            r#"<!doctype html><style>
              option { color:rgb(1,2,3) }
              option:checked { color:rgb(4,5,6) }
              #target { color:rgb(7,8,9) }
              body:has(#optionB:checked) #target { color:rgb(10,11,12) }
            </style><form><select id=selectCtl autocomplete=cc-exp-month>
              <option id=optionA value=01>January</option>
              <option id=optionB value=02>February</option>
            </select></form><div id=target></div>"#,
        );
        vm.set_layout_policy(policy);
        assert_eq!(
            vm.eval(
                r#"
                selectCtl.selectedIndex=0;
                window.aStyle=getComputedStyle(optionA);
                window.bStyle=getComputedStyle(optionB);
                window.targetStyle=getComputedStyle(target);
                window.styleColors=()=>[aStyle.color,bStyle.color,targetStyle.color];
                window.selectionEvents=[];
                for (const type of ['input','change']) {
                  selectCtl.addEventListener(type,event=>{
                    // Read held declarations before selectedness or value.
                    const colors=styleColors();
                    selectionEvents.push([
                      event.type,colors,selectCtl.value,optionA.selected,optionB.selected
                    ]);
                  });
                }
                JSON.stringify(styleColors())
                "#,
            )
            .unwrap(),
            r#"["rgb(4, 5, 6)","rgb(1, 2, 3)","rgb(7, 8, 9)"]"#
        );
        let select = vm.document_runtime.get_element_by_id("selectCtl").unwrap();
        let request = RendererAutofillTriggerRequest {
            frame_id: None,
            field_id: 0,
            card: Some(RendererAutofillCreditCard {
                number: "4111111111111111".to_owned(),
                name: "Example Person".to_owned(),
                expiry_month: "02".to_owned(),
                expiry_year: "2030".to_owned(),
                cvc: "123".to_owned(),
            }),
            address: None,
        };
        let builds = vm.layout_pass_observability_for_test().1;
        assert_eq!(
            vm.trigger_autofill(select, request.clone()).unwrap(),
            RendererAutofillTriggerOutcome::Applied {
                filled_field_count: 1
            }
        );
        // Only inspect primitives saved by the real synchronous callbacks.
        // An evaluation after autofill cannot repair these observations.
        let expected = r#"[["input",["rgb(1, 2, 3)","rgb(4, 5, 6)","rgb(10, 11, 12)"],"02",false,true],["change",["rgb(1, 2, 3)","rgb(4, 5, 6)","rgb(10, 11, 12)"],"02",false,true]]"#;
        assert_eq!(
            vm.eval("JSON.stringify(selectionEvents)").unwrap(),
            expected,
            "autofill must notify styles before its input callback: {policy:?}"
        );
        let version = vm._context_host.borrow().dom_host().dom_version();
        assert_eq!(
            vm.trigger_autofill(select, request).unwrap(),
            RendererAutofillTriggerOutcome::Applied {
                filled_field_count: 1
            }
        );
        assert_eq!(vm._context_host.borrow().dom_host().dom_version(), version);
        assert_eq!(
            vm.eval("JSON.stringify(selectionEvents)").unwrap(),
            expected
        );
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            builds,
            "autofill style reads must not require layout: {policy:?}"
        );
    }
}
