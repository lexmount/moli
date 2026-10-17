use super::*;

fn assert_only_source_document_is_dirty(
    engine: &MoliStyleEngine,
    dirty_document: DomHandle,
    clean_document: DomHandle,
) {
    assert!(
        !engine
            .source_dirty_scope_reasons_for_document_for_test(dirty_document)
            .is_empty(),
        "the source owner document must carry pending style work"
    );
    assert!(
        engine
            .source_dirty_scope_reasons_for_document_for_test(clean_document)
            .is_empty(),
        "an unrelated document must remain clean"
    );
}

fn ensure_adapter_element_data(
    engine: &MoliStyleEngine,
    host: &crate::dom::native::DomHost,
    handle: crate::document_runtime::DomHandle,
) {
    engine.dom_adapter.with_bound_host(host, |adapter| {
        let element = adapter.element(host, handle).expect("element");
        unsafe {
            let _ = element.ensure_data();
        }
    });
}

fn computed_style_snapshot_for_test(
    engine: &MoliStyleEngine,
    host: &crate::dom::native::DomHost,
    document_url: &url::Url,
    handle: crate::document_runtime::DomHandle,
    inputs: &FullStyleWorldSnapshot,
) -> StyloComputedStyleSnapshot {
    let document = host
        .owner_document_handle(handle)
        .expect("test element should have an owner document");
    engine
        .computed_style_snapshot_after_style_update_with_document_context(
            host,
            document_url,
            handle,
            inputs,
            StyleSourceDocumentContext::for_root_document(document),
            document,
            StyleViewport::default(),
        )
        .expect("test element should have computed style")
}

fn retained_primary_style_for_test(
    engine: &MoliStyleEngine,
    host: &crate::dom::native::DomHost,
    handle: crate::document_runtime::DomHandle,
) -> Option<ServoArc<style::properties::ComputedValues>> {
    engine.dom_adapter.with_bound_host(host, |adapter| {
        let element = adapter.element(host, handle)?;
        let data = element.borrow_data()?;
        data.has_styles().then(|| data.styles.primary().clone())
    })
}

fn element_style_is_dirty_for_test(
    engine: &MoliStyleEngine,
    host: &crate::dom::native::DomHost,
    handle: crate::document_runtime::DomHandle,
) -> bool {
    engine.dom_adapter.with_bound_host(host, |adapter| {
        let element = adapter.element(host, handle).expect("test element");
        element
            .borrow_data()
            .is_some_and(|data| !data.hint.is_empty())
    })
}

#[path = "subtree_cache/consumed_fragments.rs"]
mod consumed_fragments;
#[path = "subtree_cache/detached_sources.rs"]
mod detached_sources;
#[path = "subtree_cache/document_ownership.rs"]
mod document_ownership;
#[path = "subtree_cache/document_sources.rs"]
mod document_sources;
#[path = "subtree_cache/lazy_styles.rs"]
mod lazy_styles;
#[path = "subtree_cache/linked_stylesheets.rs"]
mod linked_stylesheets;
#[path = "subtree_cache/retained_stylesheets.rs"]
mod retained_stylesheets;
#[path = "subtree_cache/shadow_sources.rs"]
mod shadow_sources;
#[path = "subtree_cache/style_world.rs"]
mod style_world;
#[path = "subtree_cache/viewport_cleanup.rs"]
mod viewport_cleanup;
