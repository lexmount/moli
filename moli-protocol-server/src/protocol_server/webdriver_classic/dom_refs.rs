use moli_protocol_webdriver_classic::ClassicError;

use super::super::AppState;
use super::state::{ClassicPageBoundDomReference, ClassicSessionBinding};

pub(super) fn resolve_classic_element_dom_reference(
    state: &AppState,
    binding: &ClassicSessionBinding,
    element_id: &str,
) -> Result<ClassicPageBoundDomReference, ClassicError> {
    state
        .classic_session_registry
        .lock()
        .resolve_element_reference(binding, element_id)
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
