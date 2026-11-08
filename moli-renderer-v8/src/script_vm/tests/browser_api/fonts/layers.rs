use super::selection_order::{order_font_data, order_vm};
use super::*;

#[test]
fn font_face_layers_select_loaded_fonts_independently_of_source_and_iteration_order() {
    let (narrow, wide) = order_font_data();
    for (layers, expected) in [("first, second", "160"), ("second, first", "80")] {
        for reversed in [false, true] {
            let mut vm = order_vm();
            let mut rules = [
                format!(
                    "@layer first {{ @font-face {{ font-family:OrderFace;src:url(data:font/ttf;base64,{narrow}) }} }}"
                ),
                format!(
                    "@layer second {{ @font-face {{ font-family:OrderFace;src:url(data:font/ttf;base64,{wide}) }} }}"
                ),
            ];
            if reversed {
                rules.reverse();
            }
            let css = serde_json::to_string(&format!("@layer {layers};{}{}", rules[0], rules[1]))
                .unwrap();
            vm.eval(&format!(r#"
                const style=document.createElement('style');style.textContent={css};document.head.append(style);
                globalThis.faces=[...document.fonts];
                document.fonts.load('40px OrderFace','AAAA');
            "#)).unwrap();
            assert_eq!(
                vm.eval("faces.every(face => face.status === 'loaded')")
                    .unwrap(),
                "true"
            );
            assert_eq!(
                vm.eval("String(probe.getBoundingClientRect().width)")
                    .unwrap(),
                expected,
                "layers={layers}, reversed source={reversed}"
            );
            assert_eq!(
                vm.eval("[...document.fonts].every((face,i) => face === faces[i])")
                    .unwrap(),
                "true"
            );
            let first = if reversed { &wide } else { &narrow };
            assert_eq!(
                vm.eval(&format!("faces[0].source.includes('{first}')"))
                    .unwrap(),
                "true"
            );
        }
    }
}

#[test]
fn empty_layer_edits_reorder_other_sheets_without_replacing_font_faces() {
    let mut vm = order_vm();
    let (narrow, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.order=document.createElement('style');document.head.append(order);
        for (const [layer,data] of [['first','{narrow}'],['second','{wide}']]) {{
          const style=document.createElement('style');
          style.textContent=`@layer ${{layer}} {{ @font-face {{ font-family:OrderFace;src:url(data:font/ttf;base64,${{data}}) }} }}`;
          document.head.append(style);
        }}
        globalThis.faces=[...document.fonts];document.fonts.load('40px OrderFace','AAAA');
    "#)).unwrap();
    for (edit, expected) in [
        ("void 0", "160"),
        ("order.sheet.insertRule('@layer second {}',0)", "80"),
        ("order.sheet.deleteRule(0)", "160"),
    ] {
        vm.eval(edit).unwrap();
        vm.eval("document.fonts.load('40px OrderFace','AAAA')")
            .unwrap();
        assert_eq!(
            vm.eval("String(probe.getBoundingClientRect().width)")
                .unwrap(),
            expected
        );
        assert_eq!(vm.eval("[...document.fonts].every((face,i) => face === faces[i] && face.status === 'loaded')").unwrap(), "true");
        assert_eq!(vm.document_web_font_counts_for_test().2, 2);
    }
}

#[tokio::test]
async fn native_font_layer_reordering_needs_no_javascript_font_projection() {
    use crate::native_bridge::document::take_owner_font_face_projection_count_for_test;

    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
    let (mut vm, _completions) = new_parsed_test_vm_with_loader_and_resource_completion_queue(
        "https://font-layers.test/",
        "<!doctype html><span id=probe style='display:inline-block;font:40px OrderFace'>AAAA</span>",
        &loader,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let (narrow, wide) = order_font_data();
    take_owner_font_face_projection_count_for_test();
    vm.eval(&format!(r#"
        Object.defineProperty(document,'fonts',{{get(){{throw new Error('unexpected JS projection')}}}});
        globalThis.order=document.createElement('style');document.head.append(order);
        for (const [layer,data] of [['first','{narrow}'],['second','{wide}']]) {{
          const style=document.createElement('style');
          style.textContent=`@layer ${{layer}} {{ @font-face {{ font-family:OrderFace;src:url(data:font/ttf;base64,${{data}}) }} }}`;
          document.head.append(style);
        }}
    "#)).unwrap();
    for (edit, expected) in [
        ("void 0", "160"),
        ("order.sheet.insertRule('@layer second {}',0)", "80"),
        ("order.sheet.deleteRule(0)", "160"),
    ] {
        vm.eval(edit).unwrap();
        vm.reconcile_document_font_resources();
        assert_eq!(
            vm.eval("String(probe.getBoundingClientRect().width)")
                .unwrap(),
            expected
        );
        assert_eq!(vm.document_web_font_counts_for_test(), (2, 2, 2));
    }
    assert_eq!(take_owner_font_face_projection_count_for_test(), 0);
}

#[test]
fn unlayered_font_faces_outrank_nested_layers_and_manual_faces_outrank_css() {
    let (narrow, wide) = order_font_data();
    let mut vm = order_vm();
    vm.eval(&format!(r#"
        const style=document.createElement('style');
        style.textContent=`@font-face {{ font-family:OrderFace;src:url(data:font/ttf;base64,{narrow}) }}
          @layer outer {{ @layer first, second;
            @layer second {{ @font-face {{ font-family:OrderFace;src:url(data:font/ttf;base64,{wide}) }} }}
            @layer first {{ @font-face {{ font-family:OrderFace;src:url(data:font/ttf;base64,{narrow}) }} }}
          }}`;
        document.head.append(style);document.fonts.load('40px OrderFace','AAAA');
    "#)).unwrap();
    assert_eq!(
        vm.eval("String(probe.getBoundingClientRect().width)")
            .unwrap(),
        "80"
    );
    vm.eval(&format!("document.fonts.add(new FontFace('OrderFace',Uint8Array.from(atob('{wide}'),c=>c.charCodeAt(0))))")).unwrap();
    assert_eq!(
        vm.eval("String(probe.getBoundingClientRect().width)")
            .unwrap(),
        "160"
    );
}
