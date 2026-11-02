use moli_protocol::automation::{
    AutomationContext, DevToolsDomNodeReference, FrontendProtocol, is_webdriver_bidi_node_shared_id,
};
use moli_protocol_webdriver_classic::{
    ClassicError, ClassicErrorCode, classic_error_from_devtools_error,
};

use super::super::AppState;
use super::state::{ClassicPageBoundDomReference, ClassicSessionBinding};

pub(super) async fn resolve_classic_element_dom_reference(
    state: &AppState,
    binding: &ClassicSessionBinding,
    element_id: &str,
) -> Result<ClassicPageBoundDomReference, ClassicError> {
    if !is_webdriver_bidi_node_shared_id(element_id) {
        return state
            .classic_session_registry
            .lock()
            .resolve_element_reference(binding, element_id);
    }
    // The native export retains the original session, navigable and Document.
    // Do not fall back to JS resolution: same-origin realms can see each other's
    // nodes, which says nothing about Classic's current-frame authority.
    let exported = binding
        .runtime
        .webdriver_node_reference(
            AutomationContext {
                protocol: FrontendProtocol::WebDriverClassic,
                session_id: Some(binding.session_id.as_str().into()),
                target_id: Some(binding.browsing_context_target_id().into()),
                browser_context_id: None,
            },
            element_id.to_owned(),
        )
        .await
        .map_err(classic_error_from_devtools_error)?
        .filter(|node| node.is_element && node.target_id == binding.target_id)
        .ok_or_else(|| {
            ClassicError::new(
                ClassicErrorCode::NoSuchElement,
                "element not found in the current browsing context",
            )
        })?;
    let reference = ClassicPageBoundDomReference {
        page_residence: exported.page_residence,
        reference: DevToolsDomNodeReference::BackendNodeId(exported.backend_node_id),
    };
    state
        .classic_session_registry
        .lock()
        .register_bidi_element_reference(
            binding,
            element_id,
            exported.backend_node_id,
            reference.clone(),
        );
    Ok(reference)
}

pub(super) fn resolve_classic_shadow_root_dom_reference(
    state: &AppState,
    binding: &ClassicSessionBinding,
    shadow_root_id: &str,
) -> Result<ClassicPageBoundDomReference, ClassicError> {
    state
        .classic_session_registry
        .lock()
        .resolve_shadow_root_reference(binding, shadow_root_id)
}
