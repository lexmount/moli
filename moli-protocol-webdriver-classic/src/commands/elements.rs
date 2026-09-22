use moli_protocol::devtools_runtime::{
    DevToolsCallFunctionCommand, DevToolsCommand, DevToolsDescribeNodeCommand,
    DevToolsDomGeometryCommand, DevToolsDomGeometryOperation, DevToolsDomNodeReference,
    DevToolsEvaluateScriptCommand, DevToolsGetAttributesCommand, DevToolsGetAttributesResult,
    DevToolsGetPropertyCommand, DevToolsGetPropertyResult, DevToolsGetTextCommand,
    DevToolsGetTextResult, DevToolsLocateNodesCommand, DevToolsLocateNodesLocator,
    DevToolsLocateNodesResult, DevToolsLocateNodesTextMatch, DevToolsQuerySelectorCommand,
    DevToolsQuerySelectorResult, DevToolsReleaseObjectsCommand, DevToolsRemoteHandleId,
    DevToolsResolveNodeCommand, DevToolsResultOwnership, DevToolsScrollIntoViewIfNeededCommand,
};
use serde_json::{Value, json};

use crate::{
    CLASSIC_ELEMENT_REFERENCE_KEY, CLASSIC_SHADOW_ROOT_REFERENCE_KEY,
    ClassicDevToolsCommandContext, ClassicError, ClassicErrorCode, geometry_border_quad,
};

use super::parsing::required_string;

pub fn find_element_command(
    context: &ClassicDevToolsCommandContext,
    params: &Value,
    multiple: bool,
) -> Result<DevToolsCommand, ClassicError> {
    find_element_command_with_root(context, params, multiple, None)
}

pub fn find_element_command_with_root(
    context: &ClassicDevToolsCommandContext,
    params: &Value,
    multiple: bool,
    root: Option<DevToolsDomNodeReference>,
) -> Result<DevToolsCommand, ClassicError> {
    let using = required_string(params, "using")?;
    let value = required_string(params, "value")?;
    if using == "xpath"
        || using == "tag name"
        || using == "link text"
        || using == "partial link text"
    {
        if value.is_empty() {
            return Err(ClassicError::new(
                ClassicErrorCode::InvalidSelector,
                match using {
                    "xpath" => "xpath selector cannot be empty",
                    "tag name" => "tag name cannot be empty",
                    "link text" => "link text cannot be empty",
                    _ => "partial link text cannot be empty",
                },
            ));
        }
        return Ok(DevToolsCommand::LocateNodes(DevToolsLocateNodesCommand {
            context: context.command_context(),
            locator: match using {
                "xpath" => DevToolsLocateNodesLocator::XPath(value.to_owned()),
                "tag name" => DevToolsLocateNodesLocator::TagName(value.to_owned()),
                "link text" => DevToolsLocateNodesLocator::LinkText {
                    value: value.to_owned(),
                    match_type: DevToolsLocateNodesTextMatch::Full,
                },
                _ => DevToolsLocateNodesLocator::LinkText {
                    value: value.to_owned(),
                    match_type: DevToolsLocateNodesTextMatch::Partial,
                },
            },
            max_node_count: (!multiple).then_some(1),
            start_nodes: Vec::new(),
            start_node_references: root.into_iter().collect(),
            serialization_options: None,
        }));
    }
    let selector = classic_locator_selector(using, value)?;
    if selector.is_empty() {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "value must be a non-empty selector",
        ));
    }
    Ok(DevToolsCommand::QuerySelector(
        DevToolsQuerySelectorCommand {
            context: context.command_context(),
            root,
            selector,
            multiple,
        },
    ))
}

fn classic_locator_selector(using: &str, value: &str) -> Result<String, ClassicError> {
    match using {
        "css selector" => {
            if value.is_empty() {
                return Err(ClassicError::new(
                    ClassicErrorCode::InvalidSelector,
                    "selector cannot be empty",
                ));
            }
            Ok(value.to_owned())
        }
        "id" => Ok(format!(r#"[id="{}"]"#, css_string_escape(value))),
        "name" => Ok(format!(r#"[name="{}"]"#, css_string_escape(value))),
        "class name" => {
            if value.is_empty() {
                return Err(ClassicError::new(
                    ClassicErrorCode::InvalidSelector,
                    "class name cannot be empty",
                ));
            }
            if value.split_whitespace().count() != 1 || value.trim() != value {
                return Err(ClassicError::new(
                    ClassicErrorCode::InvalidSelector,
                    "compound class names are not allowed",
                ));
            }
            Ok(format!(r#"[class~="{}"]"#, css_string_escape(value)))
        }
        _ => Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "unsupported locator strategy",
        )),
    }
}

fn css_string_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str(r"\\"),
            '"' => escaped.push_str(r#"\""#),
            '\n' => escaped.push_str(r"\a "),
            '\r' => escaped.push_str(r"\d "),
            '\t' => escaped.push_str(r"\9 "),
            '\x0c' => escaped.push_str(r"\c "),
            _ => escaped.push(ch),
        }
    }
    escaped
}

pub fn classic_element_reference(node_id: u32) -> Value {
    json!({
        CLASSIC_ELEMENT_REFERENCE_KEY: classic_element_id(node_id),
    })
}

pub fn classic_element_id(node_id: u32) -> String {
    format!("moli-node-{node_id}")
}

pub fn classic_shadow_root_reference(shadow_root_id: impl Into<String>) -> Value {
    json!({
        CLASSIC_SHADOW_ROOT_REFERENCE_KEY: shadow_root_id.into(),
    })
}

pub fn classic_shadow_root_id(node_id: u32) -> String {
    format!("moli-shadow-{node_id}")
}

pub fn cdp_node_id_from_classic_element_id(element_id: &str) -> Result<u32, ClassicError> {
    element_id
        .strip_prefix("moli-node-")
        .and_then(classic_element_node_id_part)
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| ClassicError::new(ClassicErrorCode::NoSuchElement, "element not found"))
}

pub fn cdp_node_id_from_classic_shadow_root_id(shadow_root_id: &str) -> Result<u32, ClassicError> {
    shadow_root_id
        .strip_prefix("moli-shadow-")
        .and_then(classic_shadow_root_node_id_part)
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| {
            ClassicError::new(ClassicErrorCode::NoSuchShadowRoot, "shadow root not found")
        })
}

pub fn dom_node_reference_from_classic_element_id(
    element_id: &str,
) -> Result<DevToolsDomNodeReference, ClassicError> {
    cdp_node_id_from_classic_element_id(element_id).map(DevToolsDomNodeReference::FrontendNodeId)
}

pub fn dom_node_reference_from_classic_shadow_root_id(
    shadow_root_id: &str,
) -> Result<DevToolsDomNodeReference, ClassicError> {
    cdp_node_id_from_classic_shadow_root_id(shadow_root_id)
        .map(DevToolsDomNodeReference::FrontendNodeId)
}

fn classic_element_node_id_part(value: &str) -> Option<&str> {
    if let Some((node_id, serial)) = value.split_once("-element-") {
        if node_id.is_empty()
            || serial.is_empty()
            || !serial.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        return Some(node_id);
    }
    Some(value)
}

fn classic_shadow_root_node_id_part(value: &str) -> Option<&str> {
    if let Some((node_id, serial)) = value.split_once("-shadow-") {
        if node_id.is_empty()
            || serial.is_empty()
            || !serial.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        return Some(node_id);
    }
    Some(value)
}

pub fn classic_elements_from_query_result(result: DevToolsQuerySelectorResult) -> Vec<Value> {
    result
        .node_ids
        .into_iter()
        .map(classic_element_reference)
        .collect()
}

pub fn classic_elements_from_locate_nodes_result(result: DevToolsLocateNodesResult) -> Vec<Value> {
    result
        .node_ids
        .into_iter()
        .map(classic_element_reference)
        .collect()
}

pub fn get_element_attributes_command(
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
) -> Result<DevToolsCommand, ClassicError> {
    let reference = dom_node_reference_from_classic_element_id(element_id)?;
    Ok(get_element_attributes_reference_command(context, reference))
}

pub fn get_element_attributes_reference_command(
    context: &ClassicDevToolsCommandContext,
    reference: DevToolsDomNodeReference,
) -> DevToolsCommand {
    DevToolsCommand::GetAttributes(DevToolsGetAttributesCommand {
        context: context.command_context(),
        reference,
    })
}

pub fn get_element_text_command(
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
) -> Result<DevToolsCommand, ClassicError> {
    let reference = dom_node_reference_from_classic_element_id(element_id)?;
    Ok(get_element_text_reference_command(context, reference))
}

pub fn get_element_text_reference_command(
    context: &ClassicDevToolsCommandContext,
    reference: DevToolsDomNodeReference,
) -> DevToolsCommand {
    DevToolsCommand::GetText(DevToolsGetTextCommand {
        context: context.command_context(),
        reference,
    })
}

pub fn get_element_property_command(
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
    name: impl Into<String>,
) -> Result<DevToolsCommand, ClassicError> {
    let reference = dom_node_reference_from_classic_element_id(element_id)?;
    Ok(get_element_property_reference_command(
        context, reference, name,
    ))
}

pub fn get_element_property_reference_command(
    context: &ClassicDevToolsCommandContext,
    reference: DevToolsDomNodeReference,
    name: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::GetProperty(DevToolsGetPropertyCommand {
        context: context.command_context(),
        reference,
        name: name.into(),
    })
}

pub fn get_element_css_value_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
    property_name: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(object_id.into())),
        this_parameter: None,
        function_declaration: r#"function(propertyName) {
            if (!this || this.nodeType !== Node.ELEMENT_NODE) {
                throw new Error('__moli_webdriver_classic_stale_element_reference__');
            }
            const style = getComputedStyle(this);
            return style.getPropertyValue(propertyName);
        }"#
        .to_owned(),
        arguments: vec![json!(property_name.into())],
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn get_element_displayed_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(object_id.into())),
        this_parameter: None,
        function_declaration: CLASSIC_IS_DISPLAYED_FUNCTION.to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn get_element_rendered_text_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(object_id.into())),
        this_parameter: None,
        function_declaration: CLASSIC_RENDERED_TEXT_FUNCTION.to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn get_element_enabled_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(object_id.into())),
        this_parameter: None,
        function_declaration: CLASSIC_IS_ENABLED_FUNCTION.to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn get_element_computed_label_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(object_id.into())),
        this_parameter: None,
        function_declaration: CLASSIC_COMPUTED_LABEL_FUNCTION.to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn get_element_computed_role_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(object_id.into())),
        this_parameter: None,
        function_declaration: CLASSIC_COMPUTED_ROLE_FUNCTION.to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn get_element_shadow_root_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(object_id.into())),
        this_parameter: None,
        function_declaration: r#"function() {
            if (!this || this.nodeType !== Node.ELEMENT_NODE || !this.isConnected) {
                throw new Error('__moli_webdriver_classic_stale_element_reference__');
            }
            return this.shadowRoot;
        }"#
        .to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: true,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn describe_node_command(
    context: &ClassicDevToolsCommandContext,
    node_id: u32,
    depth: i32,
    pierce: bool,
) -> DevToolsCommand {
    describe_node_reference_command(
        context,
        DevToolsDomNodeReference::FrontendNodeId(node_id),
        depth,
        pierce,
    )
}

pub fn describe_node_reference_command(
    context: &ClassicDevToolsCommandContext,
    reference: DevToolsDomNodeReference,
    depth: i32,
    pierce: bool,
) -> DevToolsCommand {
    DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
        context: context.command_context(),
        reference: Some(reference),
        depth,
        pierce,
    })
}

pub fn verify_element_attached_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(object_id.into())),
        this_parameter: None,
        function_declaration: r#"function() {
            if (!this || this.nodeType !== Node.ELEMENT_NODE || !this.isConnected) {
                throw new Error('__moli_webdriver_classic_stale_element_reference__');
            }
            return true;
        }"#
        .to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn shadow_root_attached_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(object_id.into())),
        this_parameter: None,
        function_declaration: r#"function() {
            return Boolean(
                this &&
                this.nodeType === Node.DOCUMENT_FRAGMENT_NODE &&
                this.host &&
                this.host.isConnected
            );
        }"#
        .to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn get_element_tag_name_command(
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
) -> Result<DevToolsCommand, ClassicError> {
    get_element_property_command(context, element_id, "localName")
}

pub fn get_element_rect_command(
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
) -> Result<DevToolsCommand, ClassicError> {
    let reference = dom_node_reference_from_classic_element_id(element_id)?;
    Ok(get_element_rect_reference_command(context, reference))
}

pub fn get_element_rect_reference_command(
    context: &ClassicDevToolsCommandContext,
    reference: DevToolsDomNodeReference,
) -> DevToolsCommand {
    DevToolsCommand::DomGeometry(DevToolsDomGeometryCommand {
        context: context.command_context(),
        reference,
        operation: DevToolsDomGeometryOperation::GetBoxModel,
    })
}

pub fn active_element_command(context: &ClassicDevToolsCommandContext) -> DevToolsCommand {
    DevToolsCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        expression: "document.activeElement || document.body || document.documentElement"
            .to_owned(),
        await_promise: true,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        preserve_remote_metadata: true,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn resolve_element_command(
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
    object_group: impl Into<String>,
) -> Result<DevToolsCommand, ClassicError> {
    let reference = dom_node_reference_from_classic_element_id(element_id)?;
    Ok(resolve_element_reference_command(
        context,
        reference,
        object_group,
    ))
}

pub fn resolve_element_reference_command(
    context: &ClassicDevToolsCommandContext,
    reference: DevToolsDomNodeReference,
    object_group: impl Into<String>,
) -> DevToolsCommand {
    resolve_element_reference_command_with_execution_context(context, reference, None, object_group)
}

pub fn resolve_element_reference_command_with_execution_context(
    context: &ClassicDevToolsCommandContext,
    reference: DevToolsDomNodeReference,
    execution_context_id: Option<i64>,
    object_group: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::ResolveNode(DevToolsResolveNodeCommand {
        context: context.command_context(),
        reference,
        execution_context_id,
        object_group: Some(object_group.into()),
    })
}

pub fn resolve_shadow_root_command(
    context: &ClassicDevToolsCommandContext,
    shadow_root_id: &str,
    object_group: impl Into<String>,
) -> Result<DevToolsCommand, ClassicError> {
    let reference = dom_node_reference_from_classic_shadow_root_id(shadow_root_id)?;
    Ok(resolve_shadow_root_reference_command(
        context,
        reference,
        object_group,
    ))
}

pub fn resolve_shadow_root_reference_command(
    context: &ClassicDevToolsCommandContext,
    reference: DevToolsDomNodeReference,
    object_group: impl Into<String>,
) -> DevToolsCommand {
    resolve_shadow_root_reference_command_with_execution_context(
        context,
        reference,
        None,
        object_group,
    )
}

pub fn resolve_shadow_root_reference_command_with_execution_context(
    context: &ClassicDevToolsCommandContext,
    reference: DevToolsDomNodeReference,
    execution_context_id: Option<i64>,
    object_group: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::ResolveNode(DevToolsResolveNodeCommand {
        context: context.command_context(),
        reference,
        execution_context_id,
        object_group: Some(object_group.into()),
    })
}

pub fn frame_id_for_element_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(object_id.into())),
        this_parameter: None,
        function_declaration: CLASSIC_FRAME_ID_FOR_ELEMENT_FUNCTION.to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn release_remote_object_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::ReleaseObjects(DevToolsReleaseObjectsCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        handles: vec![DevToolsRemoteHandleId::from(object_id.into())],
    })
}

const CLASSIC_FRAME_ID_FOR_ELEMENT_FUNCTION: &str = r#"function() {
  const ownerNodeId = __moliHostResolveNodeIdForObject(this);
  if (typeof ownerNodeId !== "number" || !Number.isFinite(ownerNodeId)) {
    return null;
  }
  const frameId = __moliHostChildFrameIdForOwnerNodeId(ownerNodeId);
  return typeof frameId === "string" && frameId.length ? frameId : null;
}"#;

pub fn clear_element_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(object_id.into())),
        this_parameter: None,
        function_declaration: CLASSIC_CLEAR_ELEMENT_FUNCTION.to_owned(),
        arguments: Vec::new(),
        await_promise: true,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn element_click_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(object_id.into())),
        this_parameter: None,
        function_declaration: r#"function() {
            if (!this || this.nodeType !== Node.ELEMENT_NODE || !this.isConnected) {
                throw new Error('__moli_webdriver_classic_stale_element_reference__');
            }
            if (typeof this.click !== 'function') {
                throw new Error('__moli_webdriver_classic_element_not_interactable__');
            }
            const frameElementBeforeClick = window.frameElement || null;
            HTMLElement.prototype.click.call(this);
            let detachedFrame = false;
            try {
                detachedFrame = !!(
                    frameElementBeforeClick &&
                    parent &&
                    parent.document &&
                    !parent.document.contains(frameElementBeforeClick)
                );
            } catch (_) {
                detachedFrame = false;
            }
            return { status: 'success', detachedFrame };
        }"#
        .to_owned(),
        arguments: Vec::new(),
        await_promise: true,
        user_gesture: true,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn element_click_prepare_commands(
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
) -> Result<Vec<DevToolsCommand>, ClassicError> {
    let reference = dom_node_reference_from_classic_element_id(element_id)?;
    Ok(element_click_prepare_reference_commands(context, reference))
}

pub fn element_click_prepare_reference_commands(
    context: &ClassicDevToolsCommandContext,
    reference: DevToolsDomNodeReference,
) -> Vec<DevToolsCommand> {
    vec![
        DevToolsCommand::ScrollIntoViewIfNeeded(DevToolsScrollIntoViewIfNeededCommand {
            context: context.command_context(),
            reference: Some(reference.clone()),
            rect: None,
        }),
        DevToolsCommand::DomGeometry(DevToolsDomGeometryCommand {
            context: context.command_context(),
            reference,
            operation: DevToolsDomGeometryOperation::GetBoxModel,
        }),
    ]
}

pub fn classic_attribute_value(result: DevToolsGetAttributesResult, name: &str) -> Option<String> {
    result
        .attributes
        .into_iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| {
            if classic_is_boolean_attribute_name(name) {
                "true".to_owned()
            } else {
                attribute.value
            }
        })
}

fn classic_is_boolean_attribute_name(name: &str) -> bool {
    const BOOLEAN_ATTRIBUTES: &[&str] = &[
        "allowfullscreen",
        "allowpaymentrequest",
        "allowusermedia",
        "async",
        "autofocus",
        "autoplay",
        "checked",
        "compact",
        "complete",
        "controls",
        "declare",
        "default",
        "defaultchecked",
        "defaultselected",
        "defer",
        "disabled",
        "ended",
        "formnovalidate",
        "hidden",
        "indeterminate",
        "iscontenteditable",
        "ismap",
        "itemscope",
        "loop",
        "multiple",
        "muted",
        "nohref",
        "nomodule",
        "noresize",
        "noshade",
        "novalidate",
        "nowrap",
        "open",
        "paused",
        "playsinline",
        "pubdate",
        "readonly",
        "required",
        "reversed",
        "scoped",
        "seamless",
        "seeking",
        "selected",
        "truespeed",
        "typemustmatch",
        "willvalidate",
    ];
    BOOLEAN_ATTRIBUTES
        .iter()
        .any(|attribute| name.eq_ignore_ascii_case(attribute))
}

pub fn classic_text_value(result: DevToolsGetTextResult) -> String {
    classic_normalize_rendered_text(&result.text)
}

fn classic_normalize_rendered_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn classic_property_value(result: DevToolsGetPropertyResult) -> Value {
    result.value
}

pub fn classic_rect_from_geometry(
    geometry: &moli_protocol::devtools_runtime::DevToolsDomGeometryResult,
) -> Result<Value, ClassicError> {
    let Some(quad) = geometry_border_quad(geometry) else {
        return Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "element rect geometry did not include a quad",
        ));
    };
    if quad.points.len() < 8 {
        return Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "element rect geometry quad is incomplete",
        ));
    }
    let xs = [
        quad.points[0],
        quad.points[2],
        quad.points[4],
        quad.points[6],
    ];
    let ys = [
        quad.points[1],
        quad.points[3],
        quad.points[5],
        quad.points[7],
    ];
    let x = xs.into_iter().fold(f64::INFINITY, f64::min);
    let y = ys.into_iter().fold(f64::INFINITY, f64::min);
    let max_x = xs.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let max_y = ys.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let width = geometry.width.map(f64::from).unwrap_or_else(|| max_x - x);
    let height = geometry.height.map(f64::from).unwrap_or_else(|| max_y - y);
    if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() {
        return Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "element rect geometry contains non-finite values",
        ));
    }
    Ok(json!({
        "x": x,
        "y": y,
        "width": width,
        "height": height,
    }))
}

const CLASSIC_CLEAR_ELEMENT_FUNCTION: &str = r#"function() {
    const element = this;
    if (!element || element.nodeType !== 1) {
        return { status: 'unsupported' };
    }
    function localNameOf(value) {
        return String(value.localName || '').toLowerCase();
    }

    function isDisableable(value) {
        switch (localNameOf(value)) {
            case 'button':
            case 'input':
            case 'select':
            case 'textarea':
            case 'fieldset':
            case 'optgroup':
            case 'option':
                return true;
            default:
                return false;
        }
    }

    function isInFirstLegend(value, fieldset) {
        for (let child = fieldset.firstElementChild; child; child = child.nextElementSibling) {
            if (localNameOf(child) === 'legend') {
                return child.contains(value);
            }
        }
        return false;
    }

    function isActuallyDisabled(value) {
        if (!isDisableable(value)) {
            return false;
        }
        if (value.disabled === true || value.hasAttribute('disabled')) {
            return true;
        }
        switch (localNameOf(value)) {
            case 'button':
            case 'input':
            case 'select':
            case 'textarea':
                break;
            default:
                return false;
        }
        for (let parent = value.parentElement; parent; parent = parent.parentElement) {
            if (
                localNameOf(parent) === 'fieldset' &&
                isActuallyDisabled(parent) &&
                !isInFirstLegend(value, parent)
            ) {
                return true;
            }
        }
        return false;
    }

    const localName = localNameOf(element);
    const inputType = localName === 'input' ? String(element.type || '').toLowerCase() : '';
    const clearableInputTypes = new Set([
        'color', 'date', 'datetime-local', 'email', 'file', 'month', 'number',
        'password', 'range', 'search', 'tel', 'text', 'time', 'url', 'week'
    ]);
    const isTextControl = localName === 'textarea' ||
        (localName === 'input' && clearableInputTypes.has(inputType));
    const isContentEditable = Boolean(element.isContentEditable);
    if (!isTextControl && !isContentEditable) {
        return { status: 'unsupported' };
    }
    if (isActuallyDisabled(element) || element.readOnly) {
        return { status: 'invalid element state' };
    }
    if (isContentEditable) {
        element.textContent = '';
    } else {
        element.value = '';
    }
    element.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
    element.dispatchEvent(new Event('change', { bubbles: true }));
    return { status: 'success' };
}"#;

const CLASSIC_COMPUTED_LABEL_FUNCTION: &str = r#"function() {
    if (!this || this.nodeType !== Node.ELEMENT_NODE || !this.isConnected) {
        throw new Error('__moli_webdriver_classic_stale_element_reference__');
    }

    function normalize(value) {
        return String(value || '').replace(/\s+/g, ' ').trim();
    }

    function textAlternative(element) {
        return normalize(element && element.textContent);
    }

    const labelledBy = normalize(this.getAttribute('aria-labelledby'));
    if (labelledBy) {
        const root = this.getRootNode && this.getRootNode();
        const idOwner = root && typeof root.getElementById === 'function' ? root : this.ownerDocument;
        const parts = [];
        for (const id of labelledBy.split(/\s+/)) {
            const labelledElement = idOwner && idOwner.getElementById(id);
            const text = textAlternative(labelledElement);
            if (text) {
                parts.push(text);
            }
        }
        if (parts.length) {
            return parts.join(' ');
        }
    }

    const ariaLabel = normalize(this.getAttribute('aria-label'));
    if (ariaLabel) {
        return ariaLabel;
    }

    function labelsFor(element) {
        const labels = [];
        const id = element.getAttribute('id');
        if (id) {
            for (const label of Array.from(element.ownerDocument.getElementsByTagName('label'))) {
                if (label.getAttribute('for') === id) {
                    labels.push(label);
                }
            }
        }
        for (let ancestor = element.parentElement; ancestor; ancestor = ancestor.parentElement) {
            if (String(ancestor.localName || '').toLowerCase() === 'label') {
                labels.push(ancestor);
                break;
            }
        }
        return labels;
    }

    function labelTextFor(element) {
        const parts = [];
        for (const label of labelsFor(element)) {
            const text = textAlternative(label);
            if (text) {
                parts.push(text);
            }
        }
        if (parts.length) {
            return parts.join(' ');
        }
        return '';
    }

    const localName = String(this.localName || '').toLowerCase();
    const labelledText = labelTextFor(this);
    if (labelledText) {
        return labelledText;
    }

    if (localName === 'input') {
        const type = String(this.type || 'text').toLowerCase();
        if (type === 'button' || type === 'submit' || type === 'reset') {
            const value = normalize(this.value || this.getAttribute('value'));
            if (value) {
                return value;
            }
        }
    }

    if (localName === 'img' || localName === 'area') {
        const alt = normalize(this.getAttribute('alt'));
        if (alt) {
            return alt;
        }
    }

    if (localName === 'a' && this.hasAttribute('href')) {
        return textAlternative(this);
    }

    if (localName === 'button' || /^h[1-6]$/.test(localName)) {
        return textAlternative(this);
    }

    return '';
}"#;

const CLASSIC_COMPUTED_ROLE_FUNCTION: &str = r#"function() {
    if (!this || this.nodeType !== Node.ELEMENT_NODE || !this.isConnected) {
        throw new Error('__moli_webdriver_classic_stale_element_reference__');
    }

    const explicitRole = String(this.getAttribute('role') || '').trim().split(/\s+/)[0];
    if (explicitRole) {
        return explicitRole.toLowerCase();
    }

    const localName = String(this.localName || '').toLowerCase();
    switch (localName) {
        case 'article':
            return 'article';
        case 'button':
            return 'button';
        case 'h1':
        case 'h2':
        case 'h3':
        case 'h4':
        case 'h5':
        case 'h6':
            return 'heading';
        case 'a':
            return this.hasAttribute('href') ? 'link' : 'generic';
        case 'img':
            return 'img';
        case 'select':
            return this.multiple ? 'listbox' : 'combobox';
        case 'textarea':
            return 'textbox';
        case 'input': {
            const type = String(this.type || 'text').toLowerCase();
            switch (type) {
                case 'button':
                case 'reset':
                case 'submit':
                    return 'button';
                case 'checkbox':
                    return 'checkbox';
                case 'radio':
                    return 'radio';
                case 'range':
                    return 'slider';
                case 'search':
                    return 'searchbox';
                case 'email':
                case 'tel':
                case 'text':
                case 'url':
                case 'password':
                    return 'textbox';
                default:
                    return 'generic';
            }
        }
        default:
            return 'generic';
    }
}"#;

const CLASSIC_IS_DISPLAYED_FUNCTION: &str = r#"function() {
    if (!this || this.nodeType !== Node.ELEMENT_NODE || !this.isConnected) {
        throw new Error('__moli_webdriver_classic_stale_element_reference__');
    }
    if (this.tagName === 'INPUT' && String(this.type).toLowerCase() === 'hidden') {
        return false;
    }
    for (let node = this; node && node.nodeType === Node.ELEMENT_NODE; node = node.parentElement) {
        if (node.hidden) {
            return false;
        }
        const style = getComputedStyle(node);
        if (style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse') {
            return false;
        }
    }
    const rects = this.getClientRects();
    for (let i = 0; i < rects.length; i++) {
        if (rects[i].width > 0 && rects[i].height > 0) {
            return true;
        }
    }
    const rect = this.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
}"#;

const CLASSIC_RENDERED_TEXT_FUNCTION: &str = r#"function() {
    if (!this || this.nodeType !== Node.ELEMENT_NODE || !this.isConnected) {
        throw new Error('__moli_webdriver_classic_stale_element_reference__');
    }

    const root = this;
    const blockTags = new Set([
        'address', 'article', 'aside', 'blockquote', 'body', 'dd', 'div', 'dl', 'dt',
        'fieldset', 'figcaption', 'figure', 'footer', 'form', 'h1', 'h2', 'h3', 'h4',
        'h5', 'h6', 'header', 'hr', 'li', 'main', 'nav', 'ol', 'p', 'pre', 'section',
        'table', 'tbody', 'td', 'tfoot', 'th', 'thead', 'tr', 'ul'
    ]);
    const skippedTags = new Set(['script', 'style', 'template', 'noscript']);
    const nbspToken = Symbol('nbsp');
    const lines = [[]];

    function localName(element) {
        return String(element.localName || '').toLowerCase();
    }

    function isHiddenElement(element) {
        if (element.hasAttribute('hidden')) {
            return true;
        }
        if (localName(element) === 'input' && String(element.type || '').toLowerCase() === 'hidden') {
            return true;
        }
        const style = getComputedStyle(element);
        return style.display === 'none' ||
            style.visibility === 'hidden' ||
            style.visibility === 'collapse';
    }

    function isBlockElement(element) {
        const display = getComputedStyle(element).display;
        if (display && display !== 'inline' && display !== 'contents' && display !== 'none') {
            return true;
        }
        return blockTags.has(localName(element));
    }

    function ensureLineBreak() {
        const current = trimLine(lines[lines.length - 1]);
        if (current.length === 0) {
            lines[lines.length - 1] = [];
            return;
        }
        lines[lines.length - 1] = current;
        lines.push([]);
    }

    function isAsciiCollapsibleWhitespace(ch) {
        return ch === '\t' || ch === '\n' || ch === '\r' || ch === '\f' || ch === ' ';
    }

    function isTrimWhitespace(token) {
        return token !== nbspToken && String(token).trim() === '';
    }

    function trimLine(line) {
        let start = 0;
        let end = line.length;
        while (start < end && isTrimWhitespace(line[start])) {
            start++;
        }
        while (end > start && isTrimWhitespace(line[end - 1])) {
            end--;
        }
        return line.slice(start, end);
    }

    function appendToken(token) {
        const current = lines[lines.length - 1];
        if (token === ' ' && current[current.length - 1] === ' ') {
            return;
        }
        current.push(token);
    }

    function normalizeTextTransform(value) {
        switch (String(value || '').trim().toLowerCase()) {
            case 'none':
            case 'uppercase':
            case 'lowercase':
            case 'capitalize':
                return String(value || '').trim().toLowerCase();
            default:
                return null;
        }
    }

    function inlineTextTransform(element) {
        const style = String(element.getAttribute('style') || '');
        let transform = null;
        for (const declaration of style.split(';')) {
            const index = declaration.indexOf(':');
            if (index < 0) {
                continue;
            }
            if (declaration.slice(0, index).trim().toLowerCase() !== 'text-transform') {
                continue;
            }
            const rawValue = declaration.slice(index + 1).replace(/!important\s*$/i, '').trim();
            transform = normalizeTextTransform(rawValue) || transform;
        }
        return transform;
    }

    function textTransformFor(element, inheritedTransform) {
        const inlineTransform = inlineTextTransform(element);
        if (inlineTransform !== null) {
            return inlineTransform;
        }
        const style = getComputedStyle(element);
        const computedTransform = normalizeTextTransform(
            style.textTransform ||
            (typeof style.getPropertyValue === 'function' ? style.getPropertyValue('text-transform') : '')
        );
        if (computedTransform && computedTransform !== 'none') {
            return computedTransform;
        }
        return inheritedTransform || computedTransform || 'none';
    }

    function isTextTransformWordChar(ch) {
        return ch === '_' || /[\p{L}\p{N}]/u.test(ch);
    }

    function capitalizeText(value) {
        let out = '';
        let atWordStart = true;
        for (const ch of String(value || '')) {
            if (!isTextTransformWordChar(ch)) {
                out += ch;
                atWordStart = true;
                continue;
            }
            out += atWordStart ? ch.toUpperCase() : ch;
            atWordStart = false;
        }
        return out;
    }

    function applyTextTransform(value, transform) {
        switch (transform) {
            case 'uppercase':
                return String(value || '').toUpperCase();
            case 'lowercase':
                return String(value || '').toLowerCase();
            case 'capitalize':
                return capitalizeText(value);
            default:
                return String(value || '');
        }
    }

    function appendText(value, transform) {
        let pendingAsciiSpace = false;
        for (const ch of applyTextTransform(value, transform)) {
            if (ch === '\u00a0') {
                if (pendingAsciiSpace) {
                    appendToken(' ');
                    pendingAsciiSpace = false;
                }
                appendToken(nbspToken);
                continue;
            }
            if (isAsciiCollapsibleWhitespace(ch)) {
                pendingAsciiSpace = true;
                continue;
            }
            if (pendingAsciiSpace) {
                appendToken(' ');
                pendingAsciiSpace = false;
            }
            appendToken(ch);
        }
        if (pendingAsciiSpace) {
            appendToken(' ');
        }
    }

    function lineToString(line) {
        return line.map(token => token === nbspToken ? ' ' : token).join('');
    }

    function renderedChildren(node) {
        if (node.nodeType !== Node.ELEMENT_NODE) {
            return node.childNodes || [];
        }
        if (localName(node) === 'slot' && typeof node.assignedNodes === 'function') {
            const assigned = node.assignedNodes({ flatten: true });
            return assigned && assigned.length ? assigned : node.childNodes;
        }
        if (node.shadowRoot) {
            return node.shadowRoot.childNodes;
        }
        return node.childNodes;
    }

    function walk(node, inheritedTransform) {
        if (node.nodeType === Node.TEXT_NODE) {
            appendText(node.nodeValue, inheritedTransform);
            return;
        }
        if (node.nodeType !== Node.ELEMENT_NODE) {
            return;
        }
        const name = localName(node);
        if (skippedTags.has(name) || isHiddenElement(node)) {
            return;
        }
        if (name === 'br') {
            ensureLineBreak();
            return;
        }
        const textTransform = textTransformFor(node, inheritedTransform);
        const block = node !== root && isBlockElement(node);
        if (block) {
            ensureLineBreak();
        }
        for (const child of renderedChildren(node)) {
            walk(child, textTransform);
        }
        if (block) {
            ensureLineBreak();
        }
    }

    walk(root, 'none');
    return lines
        .map(trimLine)
        .map(lineToString)
        .filter(text => text.length > 0)
        .join('\n');
}"#;

const CLASSIC_IS_ENABLED_FUNCTION: &str = r#"function() {
    if (!this || this.nodeType !== Node.ELEMENT_NODE || !this.isConnected) {
        throw new Error('__moli_webdriver_classic_stale_element_reference__');
    }

    function localName(element) {
        return String(element.localName || '').toLowerCase();
    }

    function isDisableable(element) {
        switch (localName(element)) {
            case 'button':
            case 'input':
            case 'select':
            case 'textarea':
            case 'fieldset':
            case 'optgroup':
            case 'option':
                return true;
            default:
                return false;
        }
    }

    function isInFirstLegend(element, fieldset) {
        for (let child = fieldset.firstElementChild; child; child = child.nextElementSibling) {
            if (localName(child) === 'legend') {
                return child.contains(element);
            }
        }
        return false;
    }

    function isActuallyDisabled(element) {
        if (!isDisableable(element)) {
            return false;
        }
        if (element.disabled === true || element.hasAttribute('disabled')) {
            return true;
        }
        switch (localName(element)) {
            case 'button':
            case 'input':
            case 'select':
            case 'textarea':
                break;
            default:
                return false;
        }
        for (let parent = element.parentElement; parent; parent = parent.parentElement) {
            if (
                localName(parent) === 'fieldset' &&
                isActuallyDisabled(parent) &&
                !isInFirstLegend(element, parent)
            ) {
                return true;
            }
        }
        return false;
    }

    const name = localName(this);
    if (name === 'option' && isActuallyDisabled(this)) {
        return false;
    }
    if (name === 'option' || name === 'optgroup') {
        for (let ancestor = this; ancestor; ancestor = ancestor.parentElement) {
            const ancestorName = localName(ancestor);
            if (
                (ancestorName === 'optgroup' || ancestorName === 'select') &&
                isActuallyDisabled(ancestor)
            ) {
                return false;
            }
        }
        return true;
    }
    return !isActuallyDisabled(this);
}"#;
