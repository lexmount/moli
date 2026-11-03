use moli_protocol::automation::{
    AutomationCommand, AutomationContext, AutomationResult, DevToolsCallFunctionCommand,
    DevToolsDomNodeReference, DevToolsResultOwnership, DevToolsScriptResult, FrontendProtocol,
    is_webdriver_bidi_node_shared_id,
};
use moli_protocol_webdriver_classic::{
    ClassicError, ClassicErrorCode, classic_error_from_devtools_error,
};
use serde_json::json;

use super::super::AppState;
use super::state::{ClassicPageBoundDomReference, ClassicSessionBinding};

pub(super) async fn resolve_classic_element_dom_reference(
    state: &AppState,
    binding: &ClassicSessionBinding,
    element_id: &str,
) -> Result<ClassicPageBoundDomReference, ClassicError> {
    let registered = state
        .classic_session_registry
        .lock()
        .resolve_element_reference(binding, element_id);
    if registered.is_ok() || !is_webdriver_bidi_node_shared_id(element_id) {
        return registered;
    }
    // Resolve the renderer's session/Document-scoped BiDi binding before
    // registering a Classic alias. Never derive authority by parsing the id.
    let (result, page_residence) = binding
        .runtime
        .execute_with_page_residence(AutomationCommand::CallFunction(
            DevToolsCallFunctionCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: Some(binding.session_id.as_str().into()),
                    target_id: Some(
                        binding
                            .current_frame_id
                            .as_deref()
                            .unwrap_or(&binding.target_id)
                            .into(),
                    ),
                    browser_context_id: None,
                },
                realm_id: None,
                world_name: None,
                object_id: None,
                this_parameter: None,
                function_declaration: "function(element) { return element; }".to_owned(),
                arguments: vec![json!({"sharedId": element_id})],
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::Root,
                object_group: None,
                preserve_remote_metadata: true,
                serialization_options: None,
                materialize_bidi_script_result: true,
            },
        ))
        .await
        .map_err(|error| {
            if error.kind == moli_protocol::automation::DevToolsErrorKind::NoSuchNode {
                ClassicError::new(ClassicErrorCode::NoSuchElement, "element not found")
            } else {
                classic_error_from_devtools_error(error)
            }
        })?;
    let missing = || ClassicError::new(ClassicErrorCode::NoSuchElement, "element not found");
    let AutomationResult::Script(result) = result else {
        return Err(missing());
    };
    let DevToolsScriptResult::Value(value) = *result else {
        return Err(missing());
    };
    let context =
        moli_protocol_webdriver_classic::ClassicDevToolsCommandContext::with_protocol_and_target_id(
            FrontendProtocol::WebDriverBidi,
            &binding.session_id,
            binding
                .current_frame_id
                .as_deref()
                .unwrap_or(&binding.target_id),
        );
    super::release_classic_remote_handle(
        binding,
        &context,
        &value,
        &page_residence,
        "BiDi element reference",
    )
    .await;
    if value
        .node_value
        .as_ref()
        .and_then(|node| node["nodeType"].as_u64())
        != Some(1)
    {
        return Err(missing());
    }
    let reference = value
        .backend_node_id
        .map(DevToolsDomNodeReference::BackendNodeId)
        .or_else(|| value.node_id.map(DevToolsDomNodeReference::FrontendNodeId));
    let reference = ClassicPageBoundDomReference {
        page_residence,
        reference: reference.ok_or_else(missing)?,
    };
    state
        .classic_session_registry
        .lock()
        .register_bidi_element_reference(
            binding,
            element_id,
            value.node_id.ok_or_else(missing)?,
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
