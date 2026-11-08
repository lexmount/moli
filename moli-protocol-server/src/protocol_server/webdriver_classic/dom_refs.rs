use moli_protocol::{
    DevToolsDocumentResidenceIdentity,
    automation::{
        AutomationCommand, AutomationContext, AutomationResult, DevToolsDomNodeReference,
        DevToolsError, DevToolsErrorKind, DevToolsGetOrCreateNodeReferenceCommand,
        FrontendProtocol, WebDriverNodeKind, is_webdriver_bidi_node_shared_id,
    },
};
use moli_protocol_webdriver_classic::{
    ClassicError, ClassicErrorCode, classic_error_from_devtools_error,
};

use super::state::{ClassicPageBoundDomReference, ClassicSessionBinding};

fn node_reference_context(binding: &ClassicSessionBinding) -> AutomationContext {
    AutomationContext {
        protocol: FrontendProtocol::WebDriverClassic,
        session_id: Some(binding.session_id.as_str().into()),
        target_id: Some(binding.browsing_context_target_id().into()),
        browser_context_id: None,
    }
}

pub(super) async fn get_or_create_classic_node_reference(
    binding: &ClassicSessionBinding,
    page_residence: &DevToolsDocumentResidenceIdentity,
    reference: DevToolsDomNodeReference,
    kind: WebDriverNodeKind,
) -> Result<String, ClassicError> {
    match binding
        .runtime
        .execute_on_page(
            AutomationCommand::GetOrCreateNodeReference(DevToolsGetOrCreateNodeReferenceCommand {
                context: node_reference_context(binding),
                reference,
                kind,
            }),
            page_residence.clone(),
        )
        .await
        .map_err(|error| node_reference_error(kind, error))?
    {
        AutomationResult::NodeReference(shared_id) => Ok(shared_id.into_string()),
        _ => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "node reference export returned an unexpected result",
        )),
    }
}

fn node_reference_error(kind: WebDriverNodeKind, error: DevToolsError) -> ClassicError {
    if kind == WebDriverNodeKind::ShadowRoot && error.kind == DevToolsErrorKind::NoSuchNode {
        ClassicError::new(ClassicErrorCode::DetachedShadowRoot, error.message)
    } else {
        classic_error_from_devtools_error(error)
    }
}

async fn resolve_classic_node_reference(
    binding: &ClassicSessionBinding,
    shared_id: &str,
    kind: WebDriverNodeKind,
) -> Result<ClassicPageBoundDomReference, ClassicError> {
    let missing = || {
        ClassicError::new(
            if kind == WebDriverNodeKind::ShadowRoot {
                ClassicErrorCode::NoSuchShadowRoot
            } else {
                ClassicErrorCode::NoSuchElement
            },
            "node reference is not known in the current browsing context",
        )
    };
    if !is_webdriver_bidi_node_shared_id(shared_id) {
        return Err(missing());
    }
    let exported = binding
        .runtime
        .webdriver_node_reference(node_reference_context(binding), shared_id.to_owned(), kind)
        .await
        .map_err(|error| node_reference_error(kind, error))?
        .filter(|node| node.target_id == binding.target_id)
        .ok_or_else(missing)?;
    Ok(ClassicPageBoundDomReference {
        page_residence: exported.page_residence,
        reference: DevToolsDomNodeReference::BackendNodeId(exported.backend_node_id),
    })
}

pub(super) async fn resolve_classic_element_dom_reference(
    binding: &ClassicSessionBinding,
    element_id: &str,
) -> Result<ClassicPageBoundDomReference, ClassicError> {
    resolve_classic_node_reference(binding, element_id, WebDriverNodeKind::Element).await
}

pub(super) async fn resolve_classic_shadow_root_dom_reference(
    binding: &ClassicSessionBinding,
    shadow_root_id: &str,
) -> Result<ClassicPageBoundDomReference, ClassicError> {
    resolve_classic_node_reference(binding, shadow_root_id, WebDriverNodeKind::ShadowRoot).await
}
