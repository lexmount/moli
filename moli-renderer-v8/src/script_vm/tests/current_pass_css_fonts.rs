use super::*;
use crate::protocol_types::OptionalResourceFetchMask;
use moli_layout::{LayoutFlushReason, LayoutPassRequest};
use moli_page_types::LayoutPolicy;

fn explicit_font_width(vm: &StandaloneScriptVmHarness) -> f32 {
    let target = vm.document_runtime.get_element_by_id("target").unwrap();
    let host = vm._context_host.borrow();
    let document = host.document_handle();
    let pass = host
        .build_layout_pass_for_document(
            document,
            LayoutPassRequest::new(
                host.layout_viewport_for_document(document),
                LayoutFlushReason::Test,
            ),
        )
        .unwrap()
        .unwrap();
    assert_eq!(pass.metrics.paint_operation_count, 0);
    pass.tree.client_rects_for_source(target)[0]
        .bounding_rect()
        .width
}

#[tokio::test(flavor = "current_thread")]
async fn explicit_layout_selects_current_css_fonts_without_retiring_loaded_bytes() {
    use base64::Engine;

    let font = base64::engine::general_purpose::STANDARD.encode(include_bytes!(
        "../../../../moli-layout/tests/fixtures/moli-ahem.woff2"
    ));
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(OptionalResourceFetchMask::FONT);
    let markup = format!(
        "<!doctype html><style>@font-face{{font-family:CurrentPassFont;src:url(data:font/woff2;base64,{font})}}</style><span id=target style='display:inline-block;font:20px CurrentPassFont,serif'>iiii</span>"
    );
    let (mut vm, _completions) = new_parsed_test_vm_with_loader_and_resource_completion_queue(
        "https://current-pass-font.test/",
        &markup,
        &loader,
    );
    vm.set_layout_policy(LayoutPolicy::OnDemand);
    vm.reconcile_document_font_resources();
    assert_eq!(explicit_font_width(&vm), 48.0);
    let loaded = vm
        ._context_host
        .borrow()
        .document_web_font_counts_for_test();
    assert_eq!(loaded, (1, 1, 1));

    // Resource reconciliation and font bytes are deliberately unchanged.
    // Each explicit native pass must still use the current stylesheet input.
    vm.eval("globalThis.fontRule=document.styleSheets[0].cssRules[0].cssText;document.styleSheets[0].deleteRule(0)")
        .unwrap();
    let removed = explicit_font_width(&vm);
    assert_ne!(removed, 48.0);
    assert_eq!(
        vm._context_host
            .borrow()
            .document_web_font_counts_for_test(),
        loaded
    );

    vm.eval("document.styleSheets[0].insertRule(fontRule,0)")
        .unwrap();
    assert_eq!(explicit_font_width(&vm), 48.0);
    assert_eq!(
        vm._context_host
            .borrow()
            .document_web_font_counts_for_test(),
        loaded
    );
}
