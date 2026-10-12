use super::*;
use crate::{
    css_resource_urls::{
        CompletedStylesheetWebFont, StylesheetLoadBlockingResource,
        stylesheet_load_blocking_font_resources,
    },
    protocol_types::OptionalResourceFetchMask,
    script_vm::web_fonts::DocumentWebFontCompletion,
};
use moli_page_types::LayoutPolicy;

fn font_resource() -> StylesheetLoadBlockingResource {
    stylesheet_load_blocking_font_resources(
        "@font-face{font-family:RealtimeSourceFont;src:url(font.woff2)}",
        &Url::parse("https://realtime-font.test/style.css").unwrap(),
        OptionalResourceFetchMask::FONT,
    )
    .into_iter()
    .next()
    .unwrap()
}

fn font_response(resource: &StylesheetLoadBlockingResource) -> CompletedStylesheetWebFont {
    let (_, request) = resource.clone().into_parts();
    CompletedStylesheetWebFont::response(
        request.unwrap(),
        include_bytes!("../../../../moli-layout/tests/fixtures/moli-ahem.woff2").to_vec(),
    )
}

#[test]
fn realtime_layout_font_completion_and_revocation_refresh_without_dom_writes() {
    let mut vm = new_parsed_test_vm(
        "https://realtime-font.test/",
        "<!doctype html><span id=target style='display:inline-block;font:20px RealtimeSourceFont,serif'>iiii</span>",
    );
    vm.set_layout_policy(LayoutPolicy::Realtime);
    let resource = vm
        ._context_host
        .borrow()
        .admit_document_web_font(font_resource())
        .unwrap();
    let fallback = vm.eval("String(target.offsetWidth)").unwrap();
    assert_ne!(
        fallback, "48",
        "the fixture must start with fallback advances"
    );
    let before = vm.layout_pass_observability_for_test().1;
    let dom_version = vm._context_host.borrow().dom_host().dom_version();

    assert!(matches!(
        vm._context_host
            .borrow()
            .complete_document_web_font(font_response(&resource)),
        DocumentWebFontCompletion::Registered(moli_layout::WebFontRegistrationOutcome::Added)
    ));
    assert_eq!(
        vm._context_host.borrow().dom_host().dom_version(),
        dom_version
    );
    // The project font has a 600/1000-em advance: 4 glyphs * 20px * 0.6.
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "48");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);

    assert!(matches!(
        vm._context_host
            .borrow()
            .complete_document_web_font(font_response(&resource)),
        DocumentWebFontCompletion::Registered(moli_layout::WebFontRegistrationOutcome::Unchanged)
    ));
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "48");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);

    vm._context_host.borrow().retain_document_web_font_slots([]);
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), fallback);
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
    assert!(matches!(
        vm._context_host
            .borrow()
            .complete_document_web_font(font_response(&resource)),
        DocumentWebFontCompletion::Stale
    ));
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), fallback);
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
    assert_eq!(
        vm._context_host.borrow().dom_host().dom_version(),
        dom_version
    );
}

#[test]
fn realtime_layout_canvas_pixels_do_not_invalidate_geometry() {
    let mut vm = new_parsed_test_vm(
        "https://realtime-canvas.test/",
        "<!doctype html><canvas id=target width=100 height=50></canvas>",
    );
    vm.set_layout_policy(LayoutPolicy::Realtime);
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    let before = vm.layout_pass_observability_for_test().1;
    vm.eval("target.getContext('2d').fillRect(0,0,20,20)")
        .unwrap();
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    assert_eq!(
        vm.layout_pass_observability_for_test()
            .3
            .unwrap()
            .paint_operation_count,
        0
    );
}
