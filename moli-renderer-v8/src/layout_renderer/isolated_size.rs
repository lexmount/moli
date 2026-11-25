use moli_layout::{
    DocumentLayoutServices, IsolatedSizeLayout, LayoutError, LayoutFlushReason, LayoutPassRequest,
};

use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

pub(crate) fn try_build_isolated_sizes(
    runtime: &PageScriptHost,
    target: DomHandle,
    services: &mut DocumentLayoutServices,
) -> Result<Option<IsolatedSizeLayout<DomHandle>>, LayoutError> {
    let document = runtime.document_handle();
    let Some(root) = runtime
        .dom_host()
        .dom()
        .document_element_handle_for_document(document)
    else {
        return Ok(None);
    };
    let viewport = runtime.layout_viewport_for_document(document);
    let source = super::source_view::NativeLayoutSourceView::with_paint_resources(
        runtime,
        root,
        super::style_resolver::layout_style_read_scope(runtime, document, viewport, None),
        false,
    );
    let request = LayoutPassRequest::new(viewport, LayoutFlushReason::SynchronousGeometry)
        .with_environment(moli_layout::LayoutEnvironment {
            scrollbars_hidden: runtime.scrollbars_hidden(),
        });
    let measure = |services: &mut DocumentLayoutServices| {
        super::style_resolver::with_layout_styles(
            runtime,
            document,
            viewport,
            None,
            None,
            |styles| {
                moli_layout::try_build_isolated_sizes(&source, styles, services, target, request)
            },
        )
    };
    if services.has_stylesheet_web_fonts() {
        let resources = runtime
            .stylesheet_resource_snapshot_for_document(document)
            .ok_or_else(|| {
                LayoutError::source_contract(
                    "isolated layout fonts",
                    "missing stylesheet resource manifest",
                )
            })?;
        services.with_stylesheet_font_declarations(
            resources.web_fonts().iter().filter_map(|resource| {
                let font = resource.web_font()?;
                Some(moli_layout::WebFontDeclaration {
                    slot: font.registration_slot(),
                    source_key: font.resource_key(),
                    face: font.face(),
                })
            }),
            measure,
        )
    } else {
        measure(services)
    }
}
