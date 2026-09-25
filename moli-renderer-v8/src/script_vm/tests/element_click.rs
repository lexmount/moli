use super::*;
use crate::runtime::{RendererElementClickTarget, RendererPointerEventProperties};

#[test]
fn native_element_click_requires_explicit_layout_without_building_it() {
    let mut vm = new_parsed_test_vm(
        "https://click-native-geometry.test/",
        "<!doctype html><button id=target style='position:absolute;left:40px;top:40px;width:120px;height:70px'>go</button>",
    );
    let target = vm.document_runtime.get_element_by_id("target").unwrap();
    let before = vm.layout_pass_observability_for_test().1;
    let error = vm.prepare_element_click(target).unwrap_err();
    let crate::runtime::RendererElementClickError::LayoutUnavailable(message) = error else {
        panic!("expected missing-layout guidance, got {error:?}");
    };
    for command in ["Page.captureScreenshot", "Page.startScreencast"] {
        assert!(message.contains(command), "{message}");
    }
    assert!(!message.contains("Page.printToPDF"), "{message}");
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    publish_layout_for_test(&mut vm);
    assert!(matches!(
        vm.prepare_element_click(target).unwrap(),
        RendererElementClickTarget::Pointer(_)
    ));
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn native_element_click_reuses_frozen_layout_after_dom_and_style_changes() {
    for mutation in [
        "target.style.left='300px'",
        "const veil=document.createElement('div');veil.style.cssText='position:fixed;inset:0;z-index:999';document.body.appendChild(veil)",
        "const style=document.createElement('style');document.head.appendChild(style);style.sheet.insertRule('button {pointer-events:none}')",
    ] {
        let mut reference = None;
        for native_sequence in [false, true] {
            let mut vm = new_parsed_test_vm(
                "https://click-snapshot.test/",
                r#"<!doctype html><html><head></head><body>
            <button id='target' style='position:absolute;left:40px;top:40px;width:120px;height:70px'>go</button>
            </body></html>"#,
            );
            vm.eval(
                r#"window.clicks=0;window.events=[];
            document.getElementById('target').onclick=()=>clicks++;
            for(const type of ['mousemove','mousedown','mouseup','click'])
                document.addEventListener(type,e=>events.push([type,e.target.id]));"#,
            )
            .expect("install click listener");
            let target = vm.document_runtime.get_element_by_id("target").unwrap();
            let before = vm.layout_pass_observability_for_test().1;
            publish_layout_for_test(&mut vm);
            assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
            let RendererElementClickTarget::Pointer(first) =
                vm.prepare_element_click(target).unwrap()
            else {
                panic!("real layout must prepare pointer input");
            };
            assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);

            vm.eval(&format!(
                "(()=>{{const target=document.getElementById('target');{mutation}}})()"
            ))
            .expect("mutate live DOM/style without requesting a fresh snapshot");
            let RendererElementClickTarget::Pointer(click) =
                vm.prepare_element_click(target).unwrap()
            else {
                panic!("real layout must prepare pointer input");
            };
            assert_eq!((click.root_x, click.root_y), (first.root_x, first.root_y));
            assert_eq!(
                vm.layout_pass_observability_for_test().1,
                before + 1,
                "warm preparation must reuse the frozen tree: {mutation}"
            );
            if native_sequence {
                vm.dispatch_prepared_element_click(click).unwrap();
            } else {
                // Both native sequences and individual mouse commands consume
                // the prepared snapshot without adding a layout demand.
                for (event, buttons) in [("mousemove", 0), ("mousedown", 1), ("mouseup", 0)] {
                    vm.dispatch_mouse_event_at_point_with_pointer(
                        click.root_x,
                        click.root_y,
                        event,
                        0,
                        Some(buttons),
                        i32::from(event != "mousemove"),
                        0.0,
                        0.0,
                        RendererPointerEventProperties::default(),
                    )
                    .unwrap();
                }
            }
            let passes = vm.layout_pass_observability_for_test().1 - before;
            assert_eq!(
                passes, 1,
                "the entire mouse sequence must reuse one snapshot: {mutation}"
            );
            let events = vm.eval("JSON.stringify([clicks,events])").unwrap();
            if let Some((existing_passes, existing_events)) = &reference {
                assert!(passes <= *existing_passes, "extra layout for {mutation}");
                assert_eq!(&events, existing_events, "{mutation}");
            } else {
                reference = Some((passes, events));
            }
        }
    }
}

#[test]
fn native_element_click_mock_preparation_does_not_build_layout() {
    let mut vm = new_parsed_test_vm(
        "https://click-mock.test/",
        "<!doctype html><html><body><button id=target>go</button></body></html>",
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
    let target = vm.document_runtime.get_element_by_id("target").unwrap();
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.prepare_element_click(target).unwrap(),
        RendererElementClickTarget::DomActivation
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
}

#[test]
fn native_element_click_does_not_publish_after_scrolling_an_offscreen_target() {
    let mut vm = new_rendered_test_vm(
        "https://click-offscreen.test/",
        "<!doctype html><button id=target style='position:absolute;top:3000px'>go</button>",
    );
    let target = vm.document_runtime.get_element_by_id("target").unwrap();
    let before = vm.layout_pass_observability_for_test().1;
    assert!(matches!(
        vm.prepare_element_click(target),
        Err(crate::runtime::RendererElementClickError::NoClickableRect)
    ));
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    publish_layout_for_test(&mut vm);
    assert!(matches!(
        vm.prepare_element_click(target).unwrap(),
        RendererElementClickTarget::Pointer(_)
    ));
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}
