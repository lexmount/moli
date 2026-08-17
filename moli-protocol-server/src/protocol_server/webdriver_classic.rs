use std::{
    collections::BTreeMap,
    fs,
    path::{Path as FsPath, PathBuf},
    time::Duration,
};

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    response::Response,
};
use moli_core::page::RendererDocumentLifecycleMilestone;
use moli_protocol::{
    DevToolsPageResidenceIdentity,
    devtools_runtime::{
        DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult, DevToolsDomGeometryResult,
        DevToolsDomNodeReference, DevToolsDownloadBehaviorSetting, DevToolsError,
        DevToolsErrorKind, DevToolsFrameId, DevToolsGetFrameOwnerCommand,
        DevToolsGetFrameOwnerResult, DevToolsGetRealmsCommand, DevToolsGetRealmsResult,
        DevToolsGetServiceWorkerLogsCommand, DevToolsGetServiceWorkerLogsResult,
        DevToolsGetTargetsCommand, DevToolsGetTargetsResult, DevToolsLayoutMetricsResult,
        DevToolsLocateNodesResult, DevToolsProtocol, DevToolsQuerySelectorResult,
        DevToolsRemoteHandleId, DevToolsRemoteValue, DevToolsScriptException, DevToolsScriptResult,
        DevToolsSessionId, DevToolsSetDownloadBehaviorCommand, DevToolsSetFileInputFilesCommand,
        DevToolsTargetId, DevToolsTargetInfo, DevToolsTargetKind, RuntimeConsoleEvent,
        RuntimeExecutionContextEvent,
    },
    version,
};
use moli_protocol_webdriver_classic::{
    CLASSIC_ELEMENT_REFERENCE_KEY, CLASSIC_FRAME_REFERENCE_KEY, CLASSIC_SHADOW_ROOT_REFERENCE_KEY,
    CLASSIC_WINDOW_REFERENCE_KEY, ClassicDevToolsCommandContext,
    ClassicElementOriginViewportPoints, ClassicError, ClassicErrorCode, ClassicPageLoadStrategy,
    ClassicPromptHandler, ClassicUnhandledPromptBehavior, ClassicViewportBounds,
    action_element_origin_ids, active_element_command, alert_handle_command, alert_text_command,
    classic_attribute_value, classic_error_from_devtools_error, classic_property_value,
    classic_rect_from_geometry, classic_shadow_root_reference, classic_text_value,
    clear_element_command, create_initial_target_command, current_url_command,
    delete_session_response, describe_node_command, describe_node_reference_command,
    element_center_from_geometry, element_click_command, element_click_input_commands,
    element_click_prepare_reference_commands, element_screenshot_command,
    element_send_keys_input_commands, element_send_keys_prepare_text_control_command,
    element_send_keys_text, execute_async_command, execute_sync_command, find_element_command,
    find_element_command_with_root, get_element_attributes_reference_command,
    get_element_computed_label_command, get_element_computed_role_command,
    get_element_css_value_command, get_element_displayed_command, get_element_enabled_command,
    get_element_property_reference_command, get_element_rect_reference_command,
    get_element_rendered_text_command, get_element_text_reference_command, history_traversal_entry,
    layout_metrics_command, matched_capabilities_from_new_session_params, navigate_command,
    navigation_history_command, new_session_response, page_load_strategy_from_capabilities,
    page_source_command, parse_timeouts, print_page_command, refresh_command,
    release_remote_object_command, resolve_element_reference_command_with_execution_context,
    resolve_shadow_root_reference_command_with_execution_context, screenshot_command,
    shadow_root_attached_command, status_response as classic_status_response, timeouts_value,
    title_command, traverse_history_command, unhandled_prompt_behavior_from_capabilities,
    verify_element_attached_command, window_handles_command, window_handles_from_targets,
};
use serde_json::{Map, Value, json};
use tokio::time::sleep;
use tracing::warn;

use super::AppState;
use super::webdriver_files::{
    downloadable_file_bytes, downloadable_file_zip_base64, selected_files_from_paths,
    unique_download_directory, uploaded_file_from_base64_zip,
};

mod alerts;
mod cookies;
mod dom_refs;
mod helpers;
mod script_refs;
mod state;
mod window;

pub(super) use alerts::{
    webdriver_classic_accept_alert, webdriver_classic_dismiss_alert,
    webdriver_classic_get_alert_text, webdriver_classic_send_alert_text,
};
pub(super) use cookies::{
    webdriver_classic_add_cookie, webdriver_classic_delete_all_cookies,
    webdriver_classic_delete_cookie, webdriver_classic_get_cookies,
    webdriver_classic_get_named_cookie,
};
use dom_refs::{resolve_classic_element_dom_reference, resolve_classic_shadow_root_dom_reference};
use helpers::{
    classic_error_into_response, classic_json_body, classic_success_into_response,
    classic_webdriver_json_response,
};
use script_refs::{
    ClassicScriptCanonicalNodeReference, ClassicScriptFrameOwnerReferenceKey,
    ClassicScriptResultReference, classic_script_canonical_dom_reference,
    classic_script_canonical_dom_reference_from_described_node,
    classic_script_frame_owner_dom_reference_from_described_node,
    classic_script_result_contains_popup_window_reference, classic_script_result_reference,
    collect_classic_script_dom_reference_node_ids,
    collect_classic_script_frame_reference_owner_keys,
};
pub(super) use state::SharedClassicSessionRegistry;
use state::{ClassicPageBoundDomReference, ClassicSessionBinding, ClassicSessionRuntimeHandle};
pub(super) use window::{
    webdriver_classic_close_window, webdriver_classic_fullscreen_window,
    webdriver_classic_get_window, webdriver_classic_get_window_handles,
    webdriver_classic_get_window_rect, webdriver_classic_maximize_window,
    webdriver_classic_minimize_window, webdriver_classic_new_window,
    webdriver_classic_set_window_rect, webdriver_classic_switch_window,
};

const CLASSIC_IMPLICIT_WAIT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const CLASSIC_SCRIPT_ARG_ELEMENT_REFERENCE: &str = "__moliClassicElementReference";
const CLASSIC_SCRIPT_ARG_SHADOW_ROOT_REFERENCE: &str = "__moliClassicShadowRootReference";
const CLASSIC_SCRIPT_ARG_FRAME_REFERENCE: &str = "__moliClassicFrameReference";
const CLASSIC_SCRIPT_ARG_WINDOW_REFERENCE: &str = "__moliClassicWindowReference";
const CLASSIC_SCRIPT_WEB_REFERENCE_MARKER: &str = "__moliClassicWebReference";
const CLASSIC_SCRIPT_WEB_REFERENCE_NODE_ID: &str = "nodeId";
const CLASSIC_SCRIPT_WEB_REFERENCE_BACKEND_NODE_ID: &str = "backendNodeId";
const CLASSIC_SCRIPT_WEB_REFERENCE_ELEMENT: &str = "element";
const CLASSIC_SCRIPT_WEB_REFERENCE_SHADOW_ROOT: &str = "shadow-root";
const CLASSIC_SCRIPT_WEB_REFERENCE_FRAME: &str = "frame";
const CLASSIC_SCRIPT_WEB_REFERENCE_WINDOW: &str = "window";
const CLASSIC_SCRIPT_WEB_REFERENCE_POPUP_WINDOW: &str = "popup-window";
const CLASSIC_SCRIPT_WEB_REFERENCE_POPUP_ID: &str = "popupId";
const CLASSIC_SCRIPT_SENTINEL_STALE_ELEMENT: &str =
    "__moli_webdriver_classic_stale_element_reference__";
const CLASSIC_SCRIPT_SENTINEL_DETACHED_SHADOW_ROOT: &str =
    "__moli_webdriver_classic_detached_shadow_root__";
const CLASSIC_SCRIPT_SENTINEL_NO_SUCH_FRAME: &str = "__moli_webdriver_classic_no_such_frame__";
const CLASSIC_SCRIPT_SENTINEL_ELEMENT_NOT_INTERACTABLE: &str =
    "__moli_webdriver_classic_element_not_interactable__";

fn classic_top_level_context(binding: &ClassicSessionBinding) -> ClassicDevToolsCommandContext {
    ClassicDevToolsCommandContext::with_target_id(&binding.session_id, &binding.target_id)
}

fn classic_top_level_devtools_context(binding: &ClassicSessionBinding) -> DevToolsCommandContext {
    DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from(binding.session_id.as_str())),
        target_id: Some(DevToolsTargetId::from(binding.target_id.as_str())),
        browser_context_id: None,
    }
}

fn classic_browsing_context(binding: &ClassicSessionBinding) -> ClassicDevToolsCommandContext {
    ClassicDevToolsCommandContext::with_target_id(
        &binding.session_id,
        binding.browsing_context_target_id(),
    )
}

fn classic_session_binding(
    state: &AppState,
    session_id: &str,
) -> Result<ClassicSessionBinding, ClassicError> {
    state
        .classic_session_registry
        .lock()
        .session_binding(session_id)
        .ok_or_else(|| ClassicError::new(ClassicErrorCode::InvalidSessionId, "session not found"))
}

async fn classic_current_browsing_context_binding(
    state: &AppState,
    session_id: &str,
) -> Result<ClassicSessionBinding, ClassicError> {
    classic_binding_with_existing_current_browsing_context(classic_session_binding(
        state, session_id,
    )?)
    .await
}

async fn classic_top_level_browsing_context_binding(
    state: &AppState,
    session_id: &str,
) -> Result<ClassicSessionBinding, ClassicError> {
    let binding =
        classic_top_level_browsing_context_binding_without_prompt_handling(state, session_id)
            .await?;
    let context = classic_top_level_context(&binding);
    classic_handle_unhandled_prompt(&binding, &context).await?;
    Ok(binding)
}

async fn classic_top_level_browsing_context_binding_without_prompt_handling(
    state: &AppState,
    session_id: &str,
) -> Result<ClassicSessionBinding, ClassicError> {
    let binding = classic_session_binding(state, session_id)?;
    ensure_classic_top_level_browsing_context_exists(&binding).await?;
    Ok(binding)
}

async fn classic_binding_with_existing_current_browsing_context(
    binding: ClassicSessionBinding,
) -> Result<ClassicSessionBinding, ClassicError> {
    ensure_classic_top_level_browsing_context_exists(&binding).await?;
    let context = classic_top_level_context(&binding);
    classic_handle_unhandled_prompt(&binding, &context).await?;
    // Match ChromeDriver's window-command boundary: settle an unhandled
    // prompt first, then wait for the current navigation before touching the
    // current frame or its renderer objects.
    classic_wait_for_current_document(&binding).await?;
    ensure_classic_current_browsing_context_exists(&binding).await?;
    Ok(binding)
}

async fn classic_wait_for_current_document(
    binding: &ClassicSessionBinding,
) -> Result<(), ClassicError> {
    let milestone = match binding.page_load_strategy {
        ClassicPageLoadStrategy::None => return Ok(()),
        ClassicPageLoadStrategy::Eager => RendererDocumentLifecycleMilestone::DomContentLoaded,
        ClassicPageLoadStrategy::Normal => RendererDocumentLifecycleMilestone::Load,
    };
    binding
        .runtime
        .wait_for_document_lifecycle(
            classic_top_level_devtools_context(binding),
            milestone,
            binding.timeouts.page_load.map(Duration::from_millis),
        )
        .await
        .map_err(|error| {
            if error.kind == DevToolsErrorKind::Timeout {
                ClassicError::new(ClassicErrorCode::Timeout, "page load timed out")
            } else {
                classic_error_from_devtools_error(error)
            }
        })
}

async fn classic_handle_unhandled_prompt(
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
) -> Result<(), ClassicError> {
    let dialog = match binding.runtime.execute(alert_text_command(context)).await {
        Ok(DevToolsCommandResult::JavaScriptDialog(dialog)) => dialog,
        Ok(_) => {
            return Err(ClassicError::new(
                ClassicErrorCode::UnknownError,
                "get alert text returned an unexpected result",
            ));
        }
        Err(error) if error.kind == DevToolsErrorKind::NoSuchAlert => return Ok(()),
        Err(error) => return Err(classic_error_from_devtools_error(error)),
    };
    let handler = binding
        .unhandled_prompt_behavior
        .handler_for_prompt_type(&dialog.dialog_type);
    match handler {
        ClassicPromptHandler::Ignore => Err(classic_unexpected_alert_open_error(&dialog.message)),
        ClassicPromptHandler::Accept { notify } | ClassicPromptHandler::Dismiss { notify } => {
            let accept = matches!(handler, ClassicPromptHandler::Accept { .. });
            match binding
                .runtime
                .execute(alert_handle_command(context, accept))
                .await
            {
                Ok(DevToolsCommandResult::Empty) => {
                    if notify {
                        Err(classic_unexpected_alert_open_error(&dialog.message))
                    } else {
                        Ok(())
                    }
                }
                Ok(_) => Err(ClassicError::new(
                    ClassicErrorCode::UnknownError,
                    "handle alert returned an unexpected result",
                )),
                Err(error) => Err(classic_error_from_devtools_error(error)),
            }
        }
    }
}

fn classic_unexpected_alert_open_error(message: &str) -> ClassicError {
    ClassicError::with_data(
        ClassicErrorCode::UnexpectedAlertOpen,
        "unexpected alert open",
        json!({ "text": message }),
    )
}

async fn ensure_classic_current_browsing_context_exists(
    binding: &ClassicSessionBinding,
) -> Result<(), ClassicError> {
    let Some(frame_id) = binding.current_frame_id.clone() else {
        return Ok(());
    };
    ensure_classic_browsing_context_exists(binding, Some(frame_id)).await
}

fn classic_reset_to_top_level_browsing_context(state: &AppState, session_id: &str) {
    state
        .classic_session_registry
        .lock()
        .set_current_frame_id(session_id, None);
}

async fn ensure_classic_top_level_browsing_context_exists(
    binding: &ClassicSessionBinding,
) -> Result<(), ClassicError> {
    let context = ClassicDevToolsCommandContext::new(&binding.session_id);
    match binding
        .runtime
        .execute(window_handles_command(&context))
        .await
    {
        Ok(DevToolsCommandResult::GetTargets(result)) => {
            if window_handles_from_targets(result)
                .iter()
                .any(|target_id| target_id == &binding.target_id)
            {
                Ok(())
            } else {
                Err(classic_no_such_window_for_missing_browsing_context())
            }
        }
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "window handles returned an unexpected result",
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

async fn ensure_classic_frame_switch_target_ready(
    binding: &ClassicSessionBinding,
    frame_id: &str,
) -> Result<(), ClassicError> {
    match binding
        .runtime
        .browsing_context_exists(
            binding.session_id.clone(),
            binding.target_id.clone(),
            Some(frame_id.to_owned()),
        )
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err(ClassicError::new(
            ClassicErrorCode::NoSuchFrame,
            "frame not found",
        )),
        Err(error) => Err(classic_no_such_frame_error(error)),
    }?;

    let frame_context =
        ClassicDevToolsCommandContext::with_target_id(&binding.session_id, frame_id);
    match binding
        .runtime
        .execute(page_source_command(&frame_context))
        .await
    {
        Ok(DevToolsCommandResult::GetOuterHtml(_)) => Ok(()),
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "frame readiness probe returned an unexpected result",
        )),
        Err(error)
            if matches!(
                error.kind,
                DevToolsErrorKind::NoSuchTarget | DevToolsErrorKind::NoSuchNode
            ) || error.message == "NoDocumentLoaded" =>
        {
            Err(ClassicError::new(
                ClassicErrorCode::NoSuchFrame,
                "frame not found",
            ))
        }
        Err(error) => Err(classic_no_such_frame_error(error)),
    }
}

async fn ensure_classic_browsing_context_exists(
    binding: &ClassicSessionBinding,
    frame_id: Option<String>,
) -> Result<(), ClassicError> {
    match binding
        .runtime
        .browsing_context_exists(
            binding.session_id.clone(),
            binding.target_id.clone(),
            frame_id,
        )
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err(classic_no_such_window_for_missing_browsing_context()),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

fn classic_no_such_window_for_missing_browsing_context() -> ClassicError {
    ClassicError::new(
        ClassicErrorCode::NoSuchWindow,
        "browsing context no longer exists",
    )
}

fn classic_element_reference_from_id(element_id: String) -> Value {
    json!({
        CLASSIC_ELEMENT_REFERENCE_KEY: element_id,
    })
}

fn registered_classic_element_reference_from_dom_reference(
    state: &AppState,
    binding: &ClassicSessionBinding,
    page_residence: &DevToolsPageResidenceIdentity,
    node_id: u32,
    reference: DevToolsDomNodeReference,
) -> Value {
    let element_id = state
        .classic_session_registry
        .lock()
        .register_element_reference(
            binding,
            node_id,
            ClassicPageBoundDomReference {
                page_residence: page_residence.clone(),
                reference,
            },
        );
    classic_element_reference_from_id(element_id)
}

fn registered_classic_element_references_from_canonical_references(
    state: &AppState,
    binding: &ClassicSessionBinding,
    page_residence: &DevToolsPageResidenceIdentity,
    references: impl IntoIterator<Item = ClassicScriptCanonicalNodeReference>,
) -> Vec<Value> {
    references
        .into_iter()
        .map(|reference| {
            registered_classic_element_reference_from_dom_reference(
                state,
                binding,
                page_residence,
                reference.node_id,
                reference.reference,
            )
        })
        .collect()
}

fn registered_classic_shadow_root_reference_from_dom_reference(
    state: &AppState,
    binding: &ClassicSessionBinding,
    page_residence: &DevToolsPageResidenceIdentity,
    node_id: u32,
    reference: DevToolsDomNodeReference,
) -> Value {
    let shadow_root_id = state
        .classic_session_registry
        .lock()
        .register_shadow_root_reference(
            binding,
            node_id,
            ClassicPageBoundDomReference {
                page_residence: page_residence.clone(),
                reference,
            },
        );
    classic_shadow_root_reference(shadow_root_id)
}

fn classic_window_reference(window_id: impl Into<String>) -> Value {
    json!({
        CLASSIC_WINDOW_REFERENCE_KEY: window_id.into(),
    })
}

fn classic_frame_reference(frame_id: impl Into<String>) -> Value {
    json!({
        CLASSIC_FRAME_REFERENCE_KEY: frame_id.into(),
    })
}

#[derive(Debug)]
struct ClassicPageBoundRemoteObject {
    page_residence: DevToolsPageResidenceIdentity,
    object_id: String,
}

async fn release_classic_page_bound_remote_object(
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    remote_object: ClassicPageBoundRemoteObject,
    label: &str,
) {
    let result = binding
        .runtime
        .execute_on_page(
            release_remote_object_command(context, remote_object.object_id),
            remote_object.page_residence,
        )
        .await;
    if let Err(error) = result
        && error.kind != DevToolsErrorKind::NoSuchNode
    {
        warn!(
            ?error,
            label, "failed to release WebDriver Classic remote object"
        );
    }
}

async fn release_classic_remote_handle(
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    value: &DevToolsRemoteValue,
    page_residence: &DevToolsPageResidenceIdentity,
    label: &str,
) {
    let object_id = value
        .handle
        .as_ref()
        .or(value.shared_id.as_ref())
        .map(|handle| handle.as_str().to_owned());
    if let Some(object_id) = object_id {
        release_classic_page_bound_remote_object(
            binding,
            context,
            ClassicPageBoundRemoteObject {
                page_residence: page_residence.clone(),
                object_id,
            },
            label,
        )
        .await;
    }
}

async fn resolve_classic_element_remote_object_from_reference(
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    reference: ClassicPageBoundDomReference,
    execution_context_id: Option<i64>,
    object_group: &str,
    label: &str,
) -> Result<ClassicPageBoundRemoteObject, ClassicError> {
    let ClassicPageBoundDomReference {
        page_residence,
        reference,
    } = reference;
    let resolve = resolve_element_reference_command_with_execution_context(
        context,
        reference,
        execution_context_id,
        object_group,
    );
    match binding
        .runtime
        .execute_on_page(resolve, page_residence.clone())
        .await
    {
        Ok(DevToolsCommandResult::ResolveNode(result)) => result
            .object
            .get("objectId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .map(|object_id| ClassicPageBoundRemoteObject {
                page_residence,
                object_id,
            })
            .ok_or_else(|| {
                ClassicError::new(
                    ClassicErrorCode::StaleElementReference,
                    "element is no longer attached to the DOM",
                )
            }),
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            format!("{label} resolve returned an unexpected result"),
        )),
        Err(error) if matches!(error.kind, DevToolsErrorKind::NoSuchNode) => {
            Err(ClassicError::new(
                ClassicErrorCode::StaleElementReference,
                "element is no longer attached to the DOM",
            ))
        }
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

async fn resolve_classic_remote_object(
    state: &AppState,
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
    object_group: &str,
    label: &str,
) -> Result<ClassicPageBoundRemoteObject, ClassicError> {
    let reference = resolve_classic_element_dom_reference(state, binding, element_id)?;
    resolve_classic_element_remote_object_from_reference(
        binding,
        context,
        reference,
        None,
        object_group,
        label,
    )
    .await
}

async fn resolve_classic_shadow_root_remote_object_from_reference(
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    reference: ClassicPageBoundDomReference,
    execution_context_id: Option<i64>,
    object_group: &str,
    label: &str,
) -> Result<ClassicPageBoundRemoteObject, ClassicError> {
    let ClassicPageBoundDomReference {
        page_residence,
        reference,
    } = reference;
    let resolve = resolve_shadow_root_reference_command_with_execution_context(
        context,
        reference,
        execution_context_id,
        object_group,
    );
    match binding
        .runtime
        .execute_on_page(resolve, page_residence.clone())
        .await
    {
        Ok(DevToolsCommandResult::ResolveNode(result)) => result
            .object
            .get("objectId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .map(|object_id| ClassicPageBoundRemoteObject {
                page_residence,
                object_id,
            })
            .ok_or_else(|| {
                ClassicError::new(
                    ClassicErrorCode::DetachedShadowRoot,
                    "shadow root is detached",
                )
            }),
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            format!("{label} resolve returned an unexpected result"),
        )),
        Err(_) => Err(ClassicError::new(
            ClassicErrorCode::DetachedShadowRoot,
            "shadow root is detached",
        )),
    }
}

async fn classic_frame_owner_dom_reference(
    binding: &ClassicSessionBinding,
    frame_id: &str,
) -> Result<ClassicPageBoundDomReference, ClassicError> {
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from(binding.session_id.as_str())),
        target_id: Some(DevToolsTargetId::from(binding.target_id.as_str())),
        browser_context_id: None,
    };
    match binding
        .runtime
        .execute_with_page_residence(DevToolsCommand::GetFrameOwner(
            DevToolsGetFrameOwnerCommand {
                context,
                frame_id: DevToolsFrameId::from(frame_id),
            },
        ))
        .await
    {
        Ok((DevToolsCommandResult::GetFrameOwner(owner), page_residence)) => {
            Ok(ClassicPageBoundDomReference {
                page_residence,
                reference: classic_dom_reference_from_frame_owner_result(owner),
            })
        }
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "frame owner lookup returned an unexpected result",
        )),
        Err(error)
            if error.kind == DevToolsErrorKind::NoSuchTarget
                || error.kind == DevToolsErrorKind::NoSuchNode
                || error.message == "Frame with the given id does not belong to the target."
                || error.message == "Frame with the given id was not found."
                || error.message == "FrameOwnerRequiresPendingChildFrameResolution" =>
        {
            Err(ClassicError::new(
                ClassicErrorCode::NoSuchFrame,
                error.message,
            ))
        }
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

fn classic_dom_reference_from_frame_owner_result(
    owner: DevToolsGetFrameOwnerResult,
) -> DevToolsDomNodeReference {
    if owner.backend_node_id > 0 {
        DevToolsDomNodeReference::BackendNodeId(owner.backend_node_id)
    } else {
        DevToolsDomNodeReference::FrontendNodeId(owner.node_id)
    }
}

async fn classic_default_execution_context_id(
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    page_residence: &DevToolsPageResidenceIdentity,
    label: &str,
) -> Result<i64, ClassicError> {
    let command = DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverClassic,
            session_id: Some(DevToolsSessionId::from(context.session_id.as_str())),
            target_id: context.target_id.as_deref().map(DevToolsTargetId::from),
            browser_context_id: None,
        },
        realm_type: Some("window".to_owned()),
    });
    match binding
        .runtime
        .execute_on_page(command, page_residence.clone())
        .await
    {
        Ok(DevToolsCommandResult::Realms(result)) => result
            .realms
            .iter()
            .find(|realm| realm.is_default == Some(true) && realm.context_id.is_some())
            .or_else(|| {
                result
                    .realms
                    .iter()
                    .find(|realm| realm.context_id.is_some())
            })
            .and_then(|realm| realm.context_id)
            .ok_or_else(|| {
                ClassicError::new(
                    ClassicErrorCode::UnknownError,
                    format!("{label} could not resolve the current execution context"),
                )
            }),
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            format!("{label} realm lookup returned an unexpected result"),
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

async fn release_classic_remote_objects(
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    remote_objects: Vec<ClassicPageBoundRemoteObject>,
    label: &str,
) {
    for remote_object in remote_objects {
        release_classic_page_bound_remote_object(binding, context, remote_object, label).await;
    }
}

struct ClassicScriptArgumentHandles {
    descriptors: Vec<Value>,
    remote_handles: Vec<ClassicPageBoundRemoteObject>,
    page_residence: Option<DevToolsPageResidenceIdentity>,
}

enum ClassicScriptArgumentReference<'a> {
    Element(&'a str),
    ShadowRoot(&'a str),
    Frame(&'a str),
    Window(&'a str),
}

enum ClassicResolvedScriptArgumentReference {
    Element(ClassicPageBoundDomReference),
    ShadowRoot(ClassicPageBoundDomReference),
    Frame(ClassicPageBoundDomReference),
    Window,
}

impl ClassicResolvedScriptArgumentReference {
    fn page_residence(&self) -> Option<&DevToolsPageResidenceIdentity> {
        match self {
            Self::Element(reference) | Self::ShadowRoot(reference) | Self::Frame(reference) => {
                Some(&reference.page_residence)
            }
            Self::Window => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClassicElementLiveIdentity {
    BackendNodeId(u32),
    FrontendNodeId(u32),
}

async fn prepare_classic_script_argument_handles(
    state: &AppState,
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    params: &Value,
    label: &'static str,
) -> Result<ClassicScriptArgumentHandles, ClassicError> {
    let descriptors = classic_script_argument_descriptors(params)?;
    let references = classic_script_argument_references(&descriptors)?;
    let mut resolved_references = Vec::with_capacity(references.len());
    for reference in references {
        let reference = match reference {
            ClassicScriptArgumentReference::Element(element_id) => {
                ClassicResolvedScriptArgumentReference::Element(
                    resolve_classic_element_dom_reference(state, binding, element_id)?,
                )
            }
            ClassicScriptArgumentReference::ShadowRoot(shadow_root_id) => {
                ClassicResolvedScriptArgumentReference::ShadowRoot(
                    resolve_classic_shadow_root_dom_reference(state, binding, shadow_root_id)?,
                )
            }
            ClassicScriptArgumentReference::Frame(frame_id) => {
                ClassicResolvedScriptArgumentReference::Frame(
                    classic_frame_owner_dom_reference(binding, frame_id).await?,
                )
            }
            ClassicScriptArgumentReference::Window(window_id) => {
                if window_id != binding.browsing_context_target_id() {
                    return Err(ClassicError::new(
                        ClassicErrorCode::NoSuchWindow,
                        "window with such id was not found",
                    ));
                }
                ClassicResolvedScriptArgumentReference::Window
            }
        };
        resolved_references.push(reference);
    }
    let mut page_residence = None;
    for reference in &resolved_references {
        let Some(reference_page) = reference.page_residence() else {
            continue;
        };
        if page_residence
            .as_ref()
            .is_some_and(|expected_page| expected_page != reference_page)
        {
            return Err(ClassicError::new(
                ClassicErrorCode::StaleElementReference,
                "script argument references belong to different Pages",
            ));
        }
        page_residence.get_or_insert_with(|| reference_page.clone());
    }
    let script_execution_context_id = if resolved_references.iter().any(|reference| {
        matches!(
            reference,
            ClassicResolvedScriptArgumentReference::Element(_)
                | ClassicResolvedScriptArgumentReference::ShadowRoot(_)
        )
    }) {
        Some(
            classic_default_execution_context_id(
                binding,
                context,
                page_residence
                    .as_ref()
                    .expect("DOM script argument must identify its Page"),
                label,
            )
            .await?,
        )
    } else {
        None
    };
    let mut remote_handles = Vec::new();
    for reference in resolved_references {
        let object_id = match reference {
            ClassicResolvedScriptArgumentReference::Element(reference) => {
                resolve_classic_element_remote_object_from_reference(
                    binding,
                    context,
                    reference,
                    script_execution_context_id,
                    "webdriver-classic-script-argument",
                    label,
                )
                .await
            }
            ClassicResolvedScriptArgumentReference::ShadowRoot(reference) => {
                match resolve_classic_shadow_root_remote_object_from_reference(
                    binding,
                    context,
                    reference,
                    script_execution_context_id,
                    "webdriver-classic-script-argument",
                    label,
                )
                .await
                {
                    Ok(remote_object) => {
                        if let Err(error) = verify_classic_shadow_root_remote_object_attached(
                            binding,
                            context,
                            &remote_object,
                            label,
                        )
                        .await
                        {
                            release_classic_page_bound_remote_object(
                                binding,
                                context,
                                remote_object,
                                label,
                            )
                            .await;
                            release_classic_remote_objects(binding, context, remote_handles, label)
                                .await;
                            return Err(error);
                        }
                        Ok(remote_object)
                    }
                    Err(error) => Err(error),
                }
            }
            ClassicResolvedScriptArgumentReference::Frame(owner_reference) => {
                let top_context = classic_top_level_context(binding);
                resolve_classic_element_remote_object_from_reference(
                    binding,
                    &top_context,
                    owner_reference,
                    None,
                    "webdriver-classic-script-argument",
                    label,
                )
                .await
            }
            ClassicResolvedScriptArgumentReference::Window => {
                continue;
            }
        };
        match object_id {
            Ok(object_id) => remote_handles.push(object_id),
            Err(error) => {
                release_classic_remote_objects(binding, context, remote_handles, label).await;
                return Err(error);
            }
        }
    }
    Ok(ClassicScriptArgumentHandles {
        descriptors,
        remote_handles,
        page_residence,
    })
}

fn classic_script_argument_descriptors(params: &Value) -> Result<Vec<Value>, ClassicError> {
    let Some(args) = params.get("args") else {
        return Ok(Vec::new());
    };
    let Some(args) = args.as_array() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "args must be an array",
        ));
    };
    args.iter()
        .map(classic_script_argument_descriptor)
        .collect()
}

fn classic_script_argument_descriptor(value: &Value) -> Result<Value, ClassicError> {
    match value {
        Value::Array(values) => values
            .iter()
            .map(classic_script_argument_descriptor)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(map) => {
            if let Some(element_id) = map.get(CLASSIC_ELEMENT_REFERENCE_KEY) {
                let Some(element_id) = element_id.as_str() else {
                    return Err(ClassicError::new(
                        ClassicErrorCode::InvalidArgument,
                        "element reference must be a string",
                    ));
                };
                return Ok(json!({
                    CLASSIC_SCRIPT_ARG_ELEMENT_REFERENCE: element_id,
                }));
            }
            if let Some(shadow_root_id) = map.get(CLASSIC_SHADOW_ROOT_REFERENCE_KEY) {
                let Some(shadow_root_id) = shadow_root_id.as_str() else {
                    return Err(ClassicError::new(
                        ClassicErrorCode::InvalidArgument,
                        "shadow root reference must be a string",
                    ));
                };
                return Ok(json!({
                    CLASSIC_SCRIPT_ARG_SHADOW_ROOT_REFERENCE: shadow_root_id,
                }));
            }
            if let Some(frame_id) = map.get(CLASSIC_FRAME_REFERENCE_KEY) {
                let Some(frame_id) = frame_id.as_str() else {
                    return Err(ClassicError::new(
                        ClassicErrorCode::InvalidArgument,
                        "frame reference must be a string",
                    ));
                };
                return Ok(json!({
                    CLASSIC_SCRIPT_ARG_FRAME_REFERENCE: frame_id,
                }));
            }
            if let Some(window_id) = map.get(CLASSIC_WINDOW_REFERENCE_KEY) {
                let Some(window_id) = window_id.as_str() else {
                    return Err(ClassicError::new(
                        ClassicErrorCode::InvalidArgument,
                        "window reference must be a string",
                    ));
                };
                return Ok(json!({
                    CLASSIC_SCRIPT_ARG_WINDOW_REFERENCE: window_id,
                }));
            }
            let mut object = Map::new();
            for (key, value) in map {
                object.insert(key.clone(), classic_script_argument_descriptor(value)?);
            }
            Ok(Value::Object(object))
        }
        _ => Ok(value.clone()),
    }
}

fn classic_script_argument_references(
    descriptors: &[Value],
) -> Result<Vec<ClassicScriptArgumentReference<'_>>, ClassicError> {
    let mut references = Vec::new();
    for descriptor in descriptors {
        collect_classic_script_argument_references(descriptor, &mut references)?;
    }
    Ok(references)
}

fn collect_classic_script_argument_references<'a>(
    value: &'a Value,
    out: &mut Vec<ClassicScriptArgumentReference<'a>>,
) -> Result<(), ClassicError> {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_classic_script_argument_references(value, out)?;
            }
        }
        Value::Object(map) => {
            if let Some(element_id) =
                classic_script_reference_id(map, CLASSIC_SCRIPT_ARG_ELEMENT_REFERENCE)?
            {
                out.push(ClassicScriptArgumentReference::Element(element_id));
                return Ok(());
            }
            if let Some(shadow_root_id) =
                classic_script_reference_id(map, CLASSIC_SCRIPT_ARG_SHADOW_ROOT_REFERENCE)?
            {
                out.push(ClassicScriptArgumentReference::ShadowRoot(shadow_root_id));
                return Ok(());
            }
            if let Some(frame_id) =
                classic_script_reference_id(map, CLASSIC_SCRIPT_ARG_FRAME_REFERENCE)?
            {
                out.push(ClassicScriptArgumentReference::Frame(frame_id));
                return Ok(());
            }
            if let Some(window_id) =
                classic_script_reference_id(map, CLASSIC_SCRIPT_ARG_WINDOW_REFERENCE)?
            {
                out.push(ClassicScriptArgumentReference::Window(window_id));
                return Ok(());
            }
            for value in map.values() {
                collect_classic_script_argument_references(value, out)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn classic_script_reference_id<'a>(
    map: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, ClassicError> {
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    value.as_str().map(Some).ok_or_else(|| {
        ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "script argument reference id must be a string",
        )
    })
}

fn apply_classic_script_argument_handles(
    command: &mut DevToolsCommand,
    handles: &ClassicScriptArgumentHandles,
) {
    let DevToolsCommand::CallFunction(command) = command else {
        return;
    };
    let user_function = command.function_declaration.clone();
    command.function_declaration = classic_script_argument_deserializer_function(user_function);
    command.arguments = std::iter::once(json!({ "value": handles.descriptors }))
        .chain(
            handles
                .remote_handles
                .iter()
                .map(|remote_object| json!({ "handle": remote_object.object_id })),
        )
        .collect();
}

async fn execute_classic_script_with_argument_page(
    binding: &ClassicSessionBinding,
    command: DevToolsCommand,
    timeout: Option<Duration>,
    expected_page: Option<DevToolsPageResidenceIdentity>,
) -> Result<(DevToolsCommandResult, DevToolsPageResidenceIdentity), DevToolsError> {
    match expected_page {
        Some(expected_page) => binding
            .runtime
            .execute_script_on_page(command, timeout, expected_page.clone())
            .await
            .map(|result| (result, expected_page)),
        None => {
            binding
                .runtime
                .execute_script_with_page_residence(command, timeout)
                .await
        }
    }
}

fn classic_script_argument_deserializer_function(user_function: String) -> String {
    format!(
        "function() {{\n\
         const __moliUserFunction = ({user_function});\n\
         const __moliArgumentDescriptors = arguments[0] || [];\n\
         const __moliReferenceHandles = Array.prototype.slice.call(arguments, 1);\n\
         function __deserialize(value) {{\n\
         if (Array.isArray(value)) return value.map(__deserialize);\n\
         if (value && typeof value === 'object') {{\n\
         if (Object.prototype.hasOwnProperty.call(value, '{CLASSIC_SCRIPT_ARG_ELEMENT_REFERENCE}')) {{\n\
         const index = __moliClassicReferenceIndex++;\n\
         return __moliReferenceHandles[index];\n\
         }}\n\
         if (Object.prototype.hasOwnProperty.call(value, '{CLASSIC_SCRIPT_ARG_SHADOW_ROOT_REFERENCE}')) {{\n\
         const index = __moliClassicReferenceIndex++;\n\
         return __moliReferenceHandles[index];\n\
         }}\n\
         if (Object.prototype.hasOwnProperty.call(value, '{CLASSIC_SCRIPT_ARG_FRAME_REFERENCE}')) {{\n\
         const index = __moliClassicReferenceIndex++;\n\
         const frameElement = __moliReferenceHandles[index];\n\
         if (!frameElement || !frameElement.contentWindow) throw new Error('{CLASSIC_SCRIPT_SENTINEL_NO_SUCH_FRAME}');\n\
         return frameElement.contentWindow;\n\
         }}\n\
         if (Object.prototype.hasOwnProperty.call(value, '{CLASSIC_SCRIPT_ARG_WINDOW_REFERENCE}')) {{\n\
         return window;\n\
         }}\n\
         const out = {{}};\n\
         for (const key of Object.keys(value)) out[key] = __deserialize(value[key]);\n\
         return out;\n\
         }}\n\
         return value;\n\
         }}\n\
         function __isElement(value) {{\n\
         return typeof Element === 'function' && value instanceof Element;\n\
         }}\n\
         function __isShadowRoot(value) {{\n\
         return typeof ShadowRoot === 'function' && value instanceof ShadowRoot;\n\
         }}\n\
         function __classicNodeType(value) {{\n\
         try {{\n\
         if (!value || typeof value !== 'object' || typeof value.nodeType !== 'number') return 0;\n\
         if (typeof Node === 'function' && value instanceof Node) return value.nodeType;\n\
         const tag = Object.prototype.toString.call(value);\n\
         if (tag === '[object Attr]' || tag === '[object Text]' || tag === '[object CDATASection]' || tag === '[object ProcessingInstruction]' || tag === '[object Comment]' || tag === '[object Document]' || tag === '[object HTMLDocument]' || tag === '[object DocumentType]') return value.nodeType;\n\
         }} catch (_) {{}}\n\
         return 0;\n\
         }}\n\
         function __serializeNode(value) {{\n\
         const nodeType = __classicNodeType(value);\n\
         if (nodeType === 9) return {{ location: __serialize(value.location) }};\n\
         if (nodeType) return {{}};\n\
         return undefined;\n\
         }}\n\
         function __webReferenceForBackendNodeId(kind, backendNodeId) {{\n\
         if (typeof backendNodeId !== 'number' || backendNodeId <= 0) return {{}};\n\
         const out = {{\n\
         '{CLASSIC_SCRIPT_WEB_REFERENCE_MARKER}': kind,\n\
         '{CLASSIC_SCRIPT_WEB_REFERENCE_BACKEND_NODE_ID}': backendNodeId,\n\
         }};\n\
         return out;\n\
         }}\n\
         function __webReference(kind, node) {{\n\
         const backendNodeId = __moliHostResolveBackendNodeIdForObject(node);\n\
         return __webReferenceForBackendNodeId(kind, backendNodeId);\n\
         }}\n\
         function __isCollection(value) {{\n\
         const iterator = typeof Symbol === 'function' ? Symbol.iterator : null;\n\
         return !!iterator && !!value && typeof value[iterator] === 'function' && 'length' in value && typeof value.length === 'number';\n\
         }}\n\
         function __serializeCollection(value) {{\n\
         const out = [];\n\
         for (let index = 0; index < value.length; ++index) out.push(__serialize(value[index]));\n\
         return out;\n\
         }}\n\
         function __serializeWithCycleGuard(value, callback) {{\n\
         if (__moliClassicSeen.has(value)) throw new TypeError('cyclic object value');\n\
         __moliClassicSeen.add(value);\n\
         try {{ return callback(); }} finally {{ __moliClassicSeen.delete(value); }}\n\
         }}\n\
         function __serializeObjectToJson(value) {{\n\
         const toJSON = value.toJSON;\n\
         if (typeof toJSON !== 'function') return undefined;\n\
         return __serializeWithCycleGuard(value, () => __serialize(toJSON.call(value)));\n\
         }}\n\
         function __isDocumentAll(value) {{\n\
         try {{ return Object.prototype.toString.call(value) === '[object HTMLAllCollection]'; }} catch (_) {{ return false; }}\n\
         }}\n\
         function __serialize(value) {{\n\
         if (value === null) return null;\n\
         if (__isDocumentAll(value)) return __serializeWithCycleGuard(value, () => __serializeCollection(value));\n\
         const type = typeof value;\n\
         if (type === 'undefined' || type === 'function' || type === 'symbol') return null;\n\
         if (type !== 'object') return value;\n\
         if (value === window) {{\n\
         return {{ '{CLASSIC_SCRIPT_WEB_REFERENCE_MARKER}': '{CLASSIC_SCRIPT_WEB_REFERENCE_WINDOW}' }};\n\
         }}\n\
         const popupId = __moliHostLightweightPopupIdForObject(value);\n\
         if (popupId) return {{ '{CLASSIC_SCRIPT_WEB_REFERENCE_MARKER}': '{CLASSIC_SCRIPT_WEB_REFERENCE_POPUP_WINDOW}', '{CLASSIC_SCRIPT_WEB_REFERENCE_POPUP_ID}': String(popupId) }};\n\
         const backendNodeId = __moliHostChildFrameOwnerBackendNodeIdForWindow(value);\n\
         if (typeof backendNodeId === 'number') {{\n\
         return __webReferenceForBackendNodeId('{CLASSIC_SCRIPT_WEB_REFERENCE_FRAME}', backendNodeId);\n\
         }}\n\
         if (__isShadowRoot(value)) {{\n\
         if (!value.host || !value.host.isConnected) throw new Error('{CLASSIC_SCRIPT_SENTINEL_DETACHED_SHADOW_ROOT}');\n\
         return __webReference('{CLASSIC_SCRIPT_WEB_REFERENCE_SHADOW_ROOT}', value);\n\
         }}\n\
         if (__isElement(value)) {{\n\
         if (!value.isConnected) throw new Error('{CLASSIC_SCRIPT_SENTINEL_STALE_ELEMENT}');\n\
         return __webReference('{CLASSIC_SCRIPT_WEB_REFERENCE_ELEMENT}', value);\n\
         }}\n\
         const nodeValue = __serializeNode(value);\n\
         if (nodeValue !== undefined) return nodeValue;\n\
         if (__moliClassicSeen.has(value)) throw new TypeError('cyclic object value');\n\
         const jsonValue = __serializeObjectToJson(value);\n\
         if (jsonValue !== undefined) return jsonValue;\n\
         if (Array.isArray(value)) return __serializeWithCycleGuard(value, () => value.map(__serialize));\n\
         if (__isCollection(value)) return __serializeWithCycleGuard(value, () => __serializeCollection(value));\n\
         __moliClassicSeen.add(value);\n\
         const out = {{}};\n\
         try {{\n\
         // WebDriver clone-an-object and ChromeDriver call_function.js enumerate inherited enumerable properties.\n\
         for (const key in value) out[key] = __serialize(value[key]);\n\
         return out;\n\
         }} finally {{\n\
         __moliClassicSeen.delete(value);\n\
         }}\n\
         }}\n\
         let __moliClassicReferenceIndex = 0;\n\
         const __moliClassicSeen = new WeakSet();\n\
         const __moliArgs = __moliArgumentDescriptors.map(__deserialize);\n\
         return Promise.resolve(__moliUserFunction.apply(this, __moliArgs)).then(__serialize);\n\
         }}"
    )
}

fn classic_script_result_value(
    state: &AppState,
    binding: &ClassicSessionBinding,
    page_residence: &DevToolsPageResidenceIdentity,
    value: Value,
    dom_references_by_node_id: &BTreeMap<u32, ClassicScriptCanonicalNodeReference>,
    frame_ids_by_owner_reference: &BTreeMap<ClassicScriptFrameOwnerReferenceKey, String>,
    popup_window_handles_by_id: &BTreeMap<u64, String>,
) -> Result<Value, ClassicError> {
    match value {
        Value::Array(values) => values
            .into_iter()
            .map(|value| {
                classic_script_result_value(
                    state,
                    binding,
                    page_residence,
                    value,
                    dom_references_by_node_id,
                    frame_ids_by_owner_reference,
                    popup_window_handles_by_id,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(map) => {
            if let Some(reference) = classic_script_result_reference(&map) {
                return Ok(match reference {
                    ClassicScriptResultReference::Element {
                        node_id,
                        backend_node_id,
                    } => {
                        let reference = classic_script_canonical_dom_reference(
                            node_id,
                            backend_node_id,
                            dom_references_by_node_id,
                        );
                        registered_classic_element_reference_from_dom_reference(
                            state,
                            binding,
                            page_residence,
                            reference.node_id,
                            reference.reference,
                        )
                    }
                    ClassicScriptResultReference::ShadowRoot {
                        node_id,
                        backend_node_id,
                    } => {
                        let reference = classic_script_canonical_dom_reference(
                            node_id,
                            backend_node_id,
                            dom_references_by_node_id,
                        );
                        registered_classic_shadow_root_reference_from_dom_reference(
                            state,
                            binding,
                            page_residence,
                            reference.node_id,
                            reference.reference,
                        )
                    }
                    ClassicScriptResultReference::Frame {
                        owner: owner_reference,
                    } => {
                        let Some(frame_id) = frame_ids_by_owner_reference.get(&owner_reference)
                        else {
                            return Err(ClassicError::new(
                                ClassicErrorCode::NoSuchFrame,
                                "frame not found",
                            ));
                        };
                        classic_frame_reference(frame_id.clone())
                    }
                    ClassicScriptResultReference::Window => {
                        if let Some(frame_id) = binding.current_frame_id.as_ref() {
                            classic_frame_reference(frame_id.clone())
                        } else {
                            classic_window_reference(binding.target_id.clone())
                        }
                    }
                    ClassicScriptResultReference::PopupWindow(popup_id) => {
                        let Some(handle) = popup_window_handles_by_id.get(&popup_id) else {
                            return Err(ClassicError::new(
                                ClassicErrorCode::NoSuchWindow,
                                "popup window handle not found",
                            ));
                        };
                        classic_window_reference(handle.clone())
                    }
                });
            }
            let mut out = Map::new();
            for (key, value) in map {
                out.insert(
                    key,
                    classic_script_result_value(
                        state,
                        binding,
                        page_residence,
                        value,
                        dom_references_by_node_id,
                        frame_ids_by_owner_reference,
                        popup_window_handles_by_id,
                    )?,
                );
            }
            Ok(Value::Object(out))
        }
        _ => Ok(value),
    }
}

async fn classic_webdriver_script_result_value(
    state: &AppState,
    binding: &ClassicSessionBinding,
    page_residence: &DevToolsPageResidenceIdentity,
    value: Value,
) -> Result<Value, ClassicError> {
    let dom_references =
        classic_script_dom_references_by_node_id(binding, page_residence, &value).await?;
    let frame_ids =
        classic_script_frame_ids_by_owner_reference(binding, page_residence, &value).await?;
    let popup_window_handles_by_id =
        classic_popup_window_handles_by_id_for_script_result(binding, &value).await?;
    classic_script_result_value(
        state,
        binding,
        page_residence,
        value,
        &dom_references,
        &frame_ids,
        &popup_window_handles_by_id,
    )
}

async fn classic_script_dom_references_by_node_id(
    binding: &ClassicSessionBinding,
    page_residence: &DevToolsPageResidenceIdentity,
    value: &Value,
) -> Result<BTreeMap<u32, ClassicScriptCanonicalNodeReference>, ClassicError> {
    let mut node_ids = Vec::new();
    collect_classic_script_dom_reference_node_ids(value, &mut node_ids);
    node_ids.sort_unstable();
    node_ids.dedup();

    let mut out = BTreeMap::new();
    let context = classic_browsing_context(binding);
    for node_id in node_ids {
        let reference = match binding
            .runtime
            .execute_on_page(
                describe_node_command(&context, node_id, 0, false),
                page_residence.clone(),
            )
            .await
        {
            Ok(DevToolsCommandResult::DescribeNode(result)) => {
                classic_script_canonical_dom_reference_from_described_node(&result.node, node_id)
            }
            Ok(_) => {
                return Err(ClassicError::new(
                    ClassicErrorCode::UnknownError,
                    "script result node lookup returned an unexpected result",
                ));
            }
            Err(error) => return Err(classic_error_from_devtools_error(error)),
        };
        out.insert(node_id, reference);
    }
    Ok(out)
}

async fn classic_script_frame_ids_by_owner_reference(
    binding: &ClassicSessionBinding,
    page_residence: &DevToolsPageResidenceIdentity,
    value: &Value,
) -> Result<BTreeMap<ClassicScriptFrameOwnerReferenceKey, String>, ClassicError> {
    let mut owner_references = Vec::new();
    collect_classic_script_frame_reference_owner_keys(value, &mut owner_references);
    owner_references.sort_unstable();
    owner_references.dedup();
    let mut out = BTreeMap::new();
    for owner_reference in owner_references {
        let owner_dom_reference =
            classic_script_frame_owner_dom_reference(binding, page_residence, owner_reference)
                .await?;
        let frame_id = binding
            .runtime
            .frame_id_for_element(
                binding.session_id.clone(),
                binding.target_id.clone(),
                binding.current_frame_id.clone(),
                owner_dom_reference,
            )
            .await
            .map_err(classic_no_such_frame_error)?;
        out.insert(owner_reference, frame_id);
    }
    Ok(out)
}

async fn classic_script_frame_owner_dom_reference(
    binding: &ClassicSessionBinding,
    page_residence: &DevToolsPageResidenceIdentity,
    owner_reference: ClassicScriptFrameOwnerReferenceKey,
) -> Result<ClassicPageBoundDomReference, ClassicError> {
    if owner_reference.has_backend_node_id() {
        return Ok(ClassicPageBoundDomReference {
            page_residence: page_residence.clone(),
            reference: owner_reference.dom_reference(),
        });
    }

    let context = classic_browsing_context(binding);
    match binding
        .runtime
        .execute_on_page(
            describe_node_command(&context, owner_reference.node_id(), 0, false),
            page_residence.clone(),
        )
        .await
    {
        Ok(DevToolsCommandResult::DescribeNode(result)) => Ok(ClassicPageBoundDomReference {
            page_residence: page_residence.clone(),
            reference: classic_script_frame_owner_dom_reference_from_described_node(
                owner_reference,
                &result.node,
            ),
        }),
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "script result frame owner lookup returned an unexpected result",
        )),
        Err(error) => Err(classic_no_such_frame_error(error)),
    }
}

fn classic_execute_script_exception_error(exception: DevToolsScriptException) -> ClassicError {
    let text = classic_script_exception_text(&exception);
    if classic_exception_text_is_webdriver_sentinel(&text, CLASSIC_SCRIPT_SENTINEL_STALE_ELEMENT) {
        return ClassicError::new(
            ClassicErrorCode::StaleElementReference,
            "stale element reference",
        );
    }
    if classic_exception_text_is_webdriver_sentinel(
        &text,
        CLASSIC_SCRIPT_SENTINEL_DETACHED_SHADOW_ROOT,
    ) {
        return ClassicError::new(ClassicErrorCode::DetachedShadowRoot, "detached shadow root");
    }
    if classic_exception_text_is_webdriver_sentinel(&text, CLASSIC_SCRIPT_SENTINEL_NO_SUCH_FRAME) {
        return ClassicError::new(ClassicErrorCode::NoSuchFrame, "no such frame");
    }
    ClassicError::new(ClassicErrorCode::JavascriptError, text)
}

fn classic_webdriver_command_exception_error(exception: DevToolsScriptException) -> ClassicError {
    let text = classic_script_exception_text(&exception);
    if classic_exception_text_is_webdriver_sentinel(&text, CLASSIC_SCRIPT_SENTINEL_STALE_ELEMENT)
        || classic_exception_text_is_webdriver_sentinel(&text, "stale element reference")
    {
        return ClassicError::new(
            ClassicErrorCode::StaleElementReference,
            "stale element reference",
        );
    }
    if classic_exception_text_is_webdriver_sentinel(
        &text,
        CLASSIC_SCRIPT_SENTINEL_ELEMENT_NOT_INTERACTABLE,
    ) || classic_exception_text_is_webdriver_sentinel(&text, "element not interactable")
    {
        return ClassicError::new(
            ClassicErrorCode::ElementNotInteractable,
            "element not interactable",
        );
    }
    ClassicError::new(ClassicErrorCode::JavascriptError, text)
}

fn classic_script_exception_text(exception: &DevToolsScriptException) -> String {
    if exception.text != "Uncaught" {
        return exception.text.clone();
    }
    if let Some(description) = exception
        .value
        .as_ref()
        .and_then(|value| value.description.as_ref())
    {
        return description.clone();
    }
    if let Some(value) = exception
        .value
        .as_ref()
        .and_then(|value| value.value.as_str())
    {
        return value.to_owned();
    }
    exception.text.clone()
}

fn classic_exception_text_is_webdriver_sentinel(text: &str, sentinel: &str) -> bool {
    let first_line = text.lines().next().unwrap_or(text).trim();
    first_line == sentinel
        || first_line.strip_prefix("Error: ") == Some(sentinel)
        || first_line.strip_prefix("Uncaught Error: ") == Some(sentinel)
        || first_line.strip_prefix("Uncaught (in promise) Error: ") == Some(sentinel)
}

pub(super) async fn webdriver_classic_status(State(state): State<AppState>) -> Response {
    let ready = state.classic_session_registry.lock().session_count() == 0;
    classic_webdriver_json_response(StatusCode::OK, classic_status_response(ready, ""))
}

pub(super) async fn webdriver_classic_new_session(
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let matched_capabilities = match matched_capabilities_from_new_session_params(&params) {
        Ok(capabilities) => capabilities,
        Err(error) => return classic_error_into_response(error),
    };
    let page_load_strategy = match page_load_strategy_from_capabilities(&matched_capabilities) {
        Ok(strategy) => strategy,
        Err(error) => return classic_error_into_response(error),
    };
    let unhandled_prompt_behavior =
        match unhandled_prompt_behavior_from_capabilities(&matched_capabilities) {
            Ok(behavior) => behavior,
            Err(error) => return classic_error_into_response(error),
        };
    let downloads_enabled = match downloads_enabled_from_capabilities(&matched_capabilities) {
        Ok(enabled) => enabled,
        Err(error) => return classic_error_into_response(error),
    };
    let session = state
        .classic_session_registry
        .lock()
        .create_session(page_load_strategy, unhandled_prompt_behavior.clone());
    let download_directory = if downloads_enabled {
        let directory = unique_download_directory(&session.session_id);
        if let Err(error) = fs::create_dir_all(&directory) {
            state
                .classic_session_registry
                .lock()
                .release_session(&session.session_id);
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::SessionNotCreated,
                format!("unable to create Selenium download directory: {error}"),
            ));
        }
        if !state
            .classic_session_registry
            .lock()
            .register_download_directory(&session.session_id, directory.clone())
        {
            let _ = fs::remove_dir_all(&directory);
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::SessionNotCreated,
                "unable to register Selenium download directory",
            ));
        }
        Some(directory)
    } else {
        None
    };
    let initial_cookies = state.cookie_profile.snapshot();
    let initial_cookie_snapshot = initial_cookies.clone();
    let initial_storage_partition = state.initial_storage_partition(initial_cookies);
    let runtime = ClassicSessionRuntimeHandle::spawn(
        initial_cookie_snapshot,
        initial_storage_partition,
        moli_core::runtime::NavigationRuntimeConfig::new(
            state.fetch_config.clone(),
            state.optional_resource_fetch_mask,
            state.subframe_loading_enabled,
            state.layout_policy,
        ),
    );
    let create_context = ClassicDevToolsCommandContext::new(session.session_id.as_str());
    let target_id = match runtime
        .execute(create_initial_target_command(&create_context))
        .await
    {
        Ok(DevToolsCommandResult::CreateTarget(result)) => result.target_id.into_string(),
        Ok(_) => {
            state
                .classic_session_registry
                .lock()
                .release_session(&session.session_id);
            let _ = runtime.shutdown().await;
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::SessionNotCreated,
                "initial target creation returned an unexpected result",
            ));
        }
        Err(error) => {
            state
                .classic_session_registry
                .lock()
                .release_session(&session.session_id);
            let _ = runtime.shutdown().await;
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::SessionNotCreated,
                error.message,
            ));
        }
    };
    if let Err(error) = runtime.set_javascript_dialog_handler_enabled(true).await {
        state
            .classic_session_registry
            .lock()
            .release_session(&session.session_id);
        let _ = runtime.shutdown().await;
        return classic_error_into_response(ClassicError::new(
            ClassicErrorCode::SessionNotCreated,
            error.message,
        ));
    }
    if let Some(directory) = download_directory.as_ref() {
        let download_behavior =
            selenium_download_behavior_command(&session.session_id, directory.as_path());
        match runtime.execute(download_behavior).await {
            Ok(DevToolsCommandResult::Empty) => {}
            Ok(_) => {
                state
                    .classic_session_registry
                    .lock()
                    .release_session(&session.session_id);
                let _ = runtime.shutdown().await;
                return classic_error_into_response(ClassicError::new(
                    ClassicErrorCode::SessionNotCreated,
                    "Selenium download behavior returned an unexpected result",
                ));
            }
            Err(error) => {
                state
                    .classic_session_registry
                    .lock()
                    .release_session(&session.session_id);
                let _ = runtime.shutdown().await;
                return classic_error_into_response(ClassicError::new(
                    ClassicErrorCode::SessionNotCreated,
                    error.message,
                ));
            }
        }
    }
    state
        .classic_session_registry
        .lock()
        .bind_runtime(&session.session_id, target_id, runtime);
    let capabilities = webdriver_classic_capabilities(
        &state,
        &session.session_id,
        page_load_strategy,
        &unhandled_prompt_behavior,
        downloads_enabled,
    );
    classic_webdriver_json_response(
        StatusCode::OK,
        new_session_response(&session.session_id, capabilities),
    )
}

pub(super) async fn webdriver_classic_delete_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let runtime = state
        .classic_session_registry
        .lock()
        .release_session(&session_id);
    if let Some(runtime) = runtime {
        let cookie_commit = runtime.shutdown().await;
        if let Err(error) = state.cookie_profile.commit_and_save(cookie_commit) {
            warn!(?error, "failed to persist Classic cookie profile");
        }
        return classic_webdriver_json_response(StatusCode::OK, delete_session_response());
    }
    classic_error_into_response(ClassicError::new(
        ClassicErrorCode::InvalidSessionId,
        "session not found",
    ))
}

pub(super) async fn webdriver_classic_upload_file(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    if let Err(error) = classic_session_binding(&state, &session_id) {
        return classic_error_into_response(error);
    }
    let Some(base64_zip) = params.get("file").and_then(Value::as_str) else {
        return classic_error_into_response(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "missing or invalid 'file'",
        ));
    };
    let path = match uploaded_file_from_base64_zip(base64_zip, &session_id) {
        Ok(path) => path,
        Err(error) => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::UnknownError,
                error,
            ));
        }
    };
    let registered = state
        .classic_session_registry
        .lock()
        .register_uploaded_file(&session_id, path.clone());
    if !registered {
        let _ = std::fs::remove_file(&path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
        return classic_error_into_response(ClassicError::new(
            ClassicErrorCode::InvalidSessionId,
            "session not found",
        ));
    }
    classic_success_into_response(json!(path.to_string_lossy().to_string()))
}

pub(super) async fn webdriver_classic_get_downloadable_files(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let directory = match classic_download_directory(&state, &session_id) {
        Ok(directory) => directory,
        Err(error) => return classic_error_into_response(error),
    };
    let names = match classic_download_file_names(&directory) {
        Ok(names) => names,
        Err(error) => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::UnknownError,
                error,
            ));
        }
    };
    classic_success_into_response(json!({ "names": names }))
}

pub(super) async fn webdriver_classic_download_file(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let directory = match classic_download_directory(&state, &session_id) {
        Ok(directory) => directory,
        Err(error) => return classic_error_into_response(error),
    };
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return classic_error_into_response(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "missing or invalid 'name'",
        ));
    };
    let name = match classic_download_file_name(name) {
        Ok(name) => name,
        Err(error) => return classic_error_into_response(error),
    };
    let path = directory.join(&name);
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => {}
        _ => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "download file not found",
            ));
        }
    }
    let bytes = match downloadable_file_bytes(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::UnknownError,
                error,
            ));
        }
    };
    let contents = match downloadable_file_zip_base64(&name, &bytes) {
        Ok(contents) => contents,
        Err(error) => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::UnknownError,
                error,
            ));
        }
    };
    classic_success_into_response(json!({ "contents": contents }))
}

pub(super) async fn webdriver_classic_delete_downloadable_files(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let directory = match classic_download_directory(&state, &session_id) {
        Ok(directory) => directory,
        Err(error) => return classic_error_into_response(error),
    };
    if let Err(error) = classic_clear_download_directory(&directory) {
        return classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            error,
        ));
    }
    classic_success_into_response(Value::Null)
}

pub(super) async fn webdriver_classic_get_service_workers(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_session_binding(&state, &session_id) {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from(binding.session_id.as_str())),
        target_id: None,
        browser_context_id: None,
    };
    let targets = match binding
        .runtime
        .execute(DevToolsCommand::GetTargets(DevToolsGetTargetsCommand {
            context: context.clone(),
            root: None,
            max_depth: None,
            filter: None,
        }))
        .await
    {
        Ok(DevToolsCommandResult::GetTargets(result)) => result,
        Ok(_) => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::UnknownError,
                "service worker target listing returned an unexpected result",
            ));
        }
        Err(error) => return classic_error_into_response(classic_error_from_devtools_error(error)),
    };
    let realms = match binding
        .runtime
        .execute(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: context.clone(),
            realm_type: Some("service-worker".to_owned()),
        }))
        .await
    {
        Ok(DevToolsCommandResult::Realms(result)) => result,
        Ok(_) => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::UnknownError,
                "service worker realm listing returned an unexpected result",
            ));
        }
        Err(error) => return classic_error_into_response(classic_error_from_devtools_error(error)),
    };
    let logs = match binding
        .runtime
        .execute(DevToolsCommand::GetServiceWorkerLogs(
            DevToolsGetServiceWorkerLogsCommand {
                context,
                target_id: None,
            },
        ))
        .await
    {
        Ok(DevToolsCommandResult::ServiceWorkerLogs(result)) => result,
        Ok(_) => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::UnknownError,
                "service worker log listing returned an unexpected result",
            ));
        }
        Err(error) => return classic_error_into_response(classic_error_from_devtools_error(error)),
    };
    classic_success_into_response(classic_service_worker_observation_value(
        targets, realms, logs,
    ))
}

fn classic_service_worker_observation_value(
    targets: DevToolsGetTargetsResult,
    realms: DevToolsGetRealmsResult,
    logs: DevToolsGetServiceWorkerLogsResult,
) -> Value {
    json!({
        "targets": targets
            .targets
            .into_iter()
            .filter(|target| target.kind == DevToolsTargetKind::ServiceWorker)
            .filter_map(classic_service_worker_target_value)
            .collect::<Vec<_>>(),
        "realms": realms
            .realms
            .into_iter()
            .filter(|realm| realm.context_type.as_deref() == Some("service-worker"))
            .filter_map(classic_service_worker_realm_value)
            .collect::<Vec<_>>(),
        "logs": logs
            .entries
            .into_iter()
            .filter_map(classic_service_worker_log_value)
            .collect::<Vec<_>>(),
    })
}

fn classic_service_worker_target_value(target: DevToolsTargetInfo) -> Option<Value> {
    let mut value = Map::new();
    value.insert(
        "targetId".to_owned(),
        json!(target.target_id?.into_string()),
    );
    value.insert("type".to_owned(), json!("service_worker"));
    value.insert("title".to_owned(), json!(target.title));
    value.insert("url".to_owned(), json!(target.url));
    value.insert("attached".to_owned(), json!(target.attached));
    if let Some(browser_context_id) = target.browser_context_id {
        value.insert(
            "browserContextId".to_owned(),
            json!(browser_context_id.into_string()),
        );
    }
    Some(Value::Object(value))
}

fn classic_service_worker_realm_value(realm: RuntimeExecutionContextEvent) -> Option<Value> {
    let mut value = Map::new();
    value.insert("realm".to_owned(), json!(realm.realm_id?.into_string()));
    value.insert("targetId".to_owned(), json!(realm.target_id?.into_string()));
    value.insert("type".to_owned(), json!("service-worker"));
    if let Some(context_id) = realm.context_id {
        value.insert("executionContextId".to_owned(), json!(context_id));
    }
    if let Some(origin) = realm.origin {
        value.insert("origin".to_owned(), json!(origin));
    }
    Some(Value::Object(value))
}

fn classic_service_worker_log_value(entry: RuntimeConsoleEvent) -> Option<Value> {
    let mut value = Map::new();
    value.insert("targetId".to_owned(), json!(entry.target_id?.into_string()));
    value.insert("type".to_owned(), json!(entry.console_type));
    value.insert("text".to_owned(), json!(entry.text));
    value.insert("args".to_owned(), json!(entry.args));
    if let Some(stack) = entry.stack {
        value.insert("stack".to_owned(), json!(stack));
    }
    if let Some(context_id) = entry.execution_context_id {
        value.insert("executionContextId".to_owned(), json!(context_id));
    }
    if let Some(timestamp) = entry.timestamp {
        value.insert("timestamp".to_owned(), json!(timestamp));
    }
    Some(Value::Object(value))
}

pub(super) async fn webdriver_classic_navigate(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let binding = match classic_top_level_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_top_level_context(&binding);
    let command = match navigate_command(
        &context,
        &params,
        binding.page_load_strategy.navigation_wait(),
    ) {
        Ok(command) => command,
        Err(error) => return classic_error_into_response(error),
    };
    match webdriver_classic_execute_page_load_command(&binding, command).await {
        Ok(DevToolsCommandResult::Navigate(_)) => {
            classic_reset_to_top_level_browsing_context(&state, &session_id);
            classic_success_into_response(Value::Null)
        }
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "navigation returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(error),
    }
}

pub(super) async fn webdriver_classic_get_url(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_top_level_context(&binding);
    match binding.runtime.execute(current_url_command(&context)).await {
        Ok(DevToolsCommandResult::GetTargets(result)) => {
            let Some(target) = result.targets.into_iter().next() else {
                return classic_error_into_response(ClassicError::new(
                    ClassicErrorCode::NoSuchWindow,
                    "current window not found",
                ));
            };
            classic_success_into_response(json!(target.url))
        }
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "current URL returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_get_title(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    webdriver_classic_evaluate_string_command(session_id, state, title_command, "title").await
}

pub(super) async fn webdriver_classic_get_timeouts(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let timeouts = match state.classic_session_registry.lock().timeouts(&session_id) {
        Some(timeouts) => timeouts,
        None => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::InvalidSessionId,
                "session not found",
            ));
        }
    };
    classic_success_into_response(timeouts_value(timeouts))
}

pub(super) async fn webdriver_classic_set_timeouts(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    if body.is_empty() {
        return classic_error_into_response(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "timeouts must be an object",
        ));
    }
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let current = match state.classic_session_registry.lock().timeouts(&session_id) {
        Some(timeouts) => timeouts,
        None => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::InvalidSessionId,
                "session not found",
            ));
        }
    };
    let timeouts = match parse_timeouts(&params, current) {
        Ok(timeouts) => timeouts,
        Err(error) => return classic_error_into_response(error),
    };
    if !state
        .classic_session_registry
        .lock()
        .set_timeouts(&session_id, timeouts)
    {
        return classic_error_into_response(ClassicError::new(
            ClassicErrorCode::InvalidSessionId,
            "session not found",
        ));
    }
    classic_success_into_response(Value::Null)
}

pub(super) async fn webdriver_classic_get_source(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    match binding.runtime.execute(page_source_command(&context)).await {
        Ok(DevToolsCommandResult::GetOuterHtml(result)) => {
            classic_success_into_response(json!(result.outer_html))
        }
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "source returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_take_screenshot(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_top_level_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_top_level_context(&binding);
    match binding.runtime.execute(screenshot_command(&context)).await {
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "screenshot returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_print_page(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let binding = match classic_top_level_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_top_level_context(&binding);
    let command = match print_page_command(&context, &params) {
        Ok(command) => command,
        Err(error) => return classic_error_into_response(error),
    };
    match binding.runtime.execute(command).await {
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "print returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

async fn webdriver_classic_evaluate_string_command(
    session_id: String,
    state: AppState,
    build_command: impl FnOnce(&ClassicDevToolsCommandContext) -> DevToolsCommand,
    label: &'static str,
) -> Response {
    let binding = match classic_top_level_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_top_level_context(&binding);
    match binding
        .runtime
        .execute_with_pending_navigation_wait(
            build_command(&context),
            None,
            binding.timeouts.page_load.map(Duration::from_millis),
        )
        .await
    {
        Ok(DevToolsCommandResult::Script(result)) => match *result {
            DevToolsScriptResult::Value(value) => {
                if !value.value.is_string() {
                    return classic_error_into_response(ClassicError::new(
                        ClassicErrorCode::UnknownError,
                        format!("{label} returned a non-string value"),
                    ));
                }
                if label == "title" {
                    let title = value.value.as_str().unwrap_or_default();
                    return classic_success_into_response(json!(classic_webdriver_title(title)));
                }
                classic_success_into_response(value.value)
            }
            DevToolsScriptResult::Exception(exception) => {
                classic_error_into_response(classic_webdriver_command_exception_error(exception))
            }
        },
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            format!("{label} returned an unexpected result"),
        )),
        Err(error) if label == "title" && error.message == "NoDocumentLoaded" => {
            classic_success_into_response(json!(""))
        }
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

fn classic_webdriver_title(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ")
}

async fn classic_current_url(binding: &ClassicSessionBinding) -> Result<String, ClassicError> {
    let context = classic_top_level_context(binding);
    match binding.runtime.execute(current_url_command(&context)).await {
        Ok(DevToolsCommandResult::GetTargets(result)) => {
            let Some(target) = result.targets.into_iter().next() else {
                return Err(ClassicError::new(
                    ClassicErrorCode::NoSuchWindow,
                    "current window not found",
                ));
            };
            Ok(target.url)
        }
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "current URL returned an unexpected result",
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_switch_frame(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let Some(frame) = params.get("id") else {
        return classic_error_into_response(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "id must be null, a non-negative integer, or a web element",
        ));
    };
    if frame.is_null() {
        let binding = match classic_top_level_browsing_context_binding_without_prompt_handling(
            &state,
            &session_id,
        )
        .await
        {
            Ok(binding) => binding,
            Err(error) => return classic_error_into_response(error),
        };
        if let Err(error) = ensure_classic_top_level_browsing_context_exists(&binding).await {
            return classic_error_into_response(error);
        }
        if !state
            .classic_session_registry
            .lock()
            .set_current_frame_id(&session_id, None)
        {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::InvalidSessionId,
                "session not found",
            ));
        }
        return classic_success_into_response(Value::Null);
    }
    let binding = match classic_top_level_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };

    let frame_id = if let Some(index) = classic_frame_index(frame) {
        // Non-null frame switches resolve relative to the current browsing context.
        // `id: null` intentionally checks only the top-level context above.
        if let Err(error) = ensure_classic_current_browsing_context_exists(&binding).await {
            return classic_error_into_response(error);
        }
        let frame_id = match binding
            .runtime
            .frame_id_for_index(
                binding.session_id.clone(),
                binding.target_id.clone(),
                binding.current_frame_id.clone(),
                index,
            )
            .await
        {
            Ok(frame_id) => frame_id,
            Err(error) => return classic_error_into_response(classic_no_such_frame_error(error)),
        };
        Some(frame_id)
    } else if let Some(element_id) = classic_frame_element_id(frame) {
        // Element-based frame switches also need the current context before
        // resolving the reference against it.
        if let Err(error) = ensure_classic_current_browsing_context_exists(&binding).await {
            return classic_error_into_response(error);
        }
        let reference = match resolve_classic_element_dom_reference(&state, &binding, element_id) {
            Ok(reference) => reference,
            Err(error) => return classic_error_into_response(error),
        };
        let frame_id = match binding
            .runtime
            .frame_id_for_element(
                binding.session_id.clone(),
                binding.target_id.clone(),
                binding.current_frame_id.clone(),
                reference,
            )
            .await
        {
            Ok(frame_id) => frame_id,
            Err(error) if error.kind == DevToolsErrorKind::NoSuchNode => {
                return classic_error_into_response(classic_error_from_devtools_error(error));
            }
            Err(error) => return classic_error_into_response(classic_no_such_frame_error(error)),
        };
        Some(frame_id)
    } else {
        return classic_error_into_response(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "id must be null, a non-negative integer, or a web element",
        ));
    };

    if let Some(frame_id) = frame_id.as_ref()
        && let Err(error) = ensure_classic_frame_switch_target_ready(&binding, frame_id).await
    {
        return classic_error_into_response(error);
    }

    if !state
        .classic_session_registry
        .lock()
        .set_current_frame_id(&session_id, frame_id)
    {
        return classic_error_into_response(ClassicError::new(
            ClassicErrorCode::InvalidSessionId,
            "session not found",
        ));
    }
    classic_success_into_response(Value::Null)
}

pub(super) async fn webdriver_classic_switch_parent_frame(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_top_level_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    if let Err(error) = ensure_classic_top_level_browsing_context_exists(&binding).await {
        return classic_error_into_response(error);
    }
    let frame_id = match binding.current_frame_id.as_deref() {
        Some(current_frame_id) => match binding
            .runtime
            .parent_frame_id(
                binding.session_id.clone(),
                binding.target_id.clone(),
                current_frame_id.to_owned(),
            )
            .await
        {
            Ok(frame_id) => frame_id,
            Err(error) if error.kind == DevToolsErrorKind::NoSuchTarget => None,
            Err(error) => return classic_error_into_response(classic_no_such_frame_error(error)),
        },
        None => None,
    };
    if !state
        .classic_session_registry
        .lock()
        .set_current_frame_id(&session_id, frame_id)
    {
        return classic_error_into_response(ClassicError::new(
            ClassicErrorCode::InvalidSessionId,
            "session not found",
        ));
    }
    classic_success_into_response(Value::Null)
}

fn classic_frame_index(value: &Value) -> Option<usize> {
    value
        .as_u64()
        .filter(|index| *index <= u16::MAX as u64)
        .and_then(|index| usize::try_from(index).ok())
}

fn classic_frame_element_id(value: &Value) -> Option<&str> {
    value
        .as_object()
        .and_then(|object| object.get(CLASSIC_ELEMENT_REFERENCE_KEY))
        .and_then(Value::as_str)
}

fn classic_no_such_frame_error(error: DevToolsError) -> ClassicError {
    match error.kind {
        DevToolsErrorKind::NoSuchTarget | DevToolsErrorKind::NoSuchNode => {
            ClassicError::new(ClassicErrorCode::NoSuchFrame, error.message)
        }
        _ => classic_error_from_devtools_error(error),
    }
}

async fn classic_popup_window_handles_by_id_for_script_result(
    binding: &ClassicSessionBinding,
    value: &Value,
) -> Result<BTreeMap<u64, String>, ClassicError> {
    if !classic_script_result_contains_popup_window_reference(value) {
        return Ok(BTreeMap::new());
    }
    let context = classic_top_level_context(binding);
    match binding
        .runtime
        .execute(window_handles_command(&context))
        .await
    {
        Ok(DevToolsCommandResult::GetTargets(result)) => Ok(result
            .targets
            .into_iter()
            .filter(|target| {
                target.kind == moli_protocol::devtools_runtime::DevToolsTargetKind::Page
            })
            .filter_map(|target| {
                let popup_id = target.moli_popup_id?;
                let target_id = target.target_id?.into_string();
                Some((popup_id, target_id))
            })
            .collect()),
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "window handles returned an unexpected result",
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_refresh(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_top_level_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_top_level_context(&binding);
    match webdriver_classic_execute_page_load_command(
        &binding,
        refresh_command(&context, binding.page_load_strategy.navigation_wait()),
    )
    .await
    {
        Ok(DevToolsCommandResult::Navigate(_)) => {
            classic_reset_to_top_level_browsing_context(&state, &session_id);
            classic_success_into_response(Value::Null)
        }
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "refresh returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(error),
    }
}

pub(super) async fn webdriver_classic_back(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    webdriver_classic_traverse_history(session_id, state, -1).await
}

pub(super) async fn webdriver_classic_forward(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    webdriver_classic_traverse_history(session_id, state, 1).await
}

async fn webdriver_classic_traverse_history(
    session_id: String,
    state: AppState,
    delta: i32,
) -> Response {
    let binding = match classic_top_level_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_top_level_context(&binding);
    let history = match binding
        .runtime
        .execute(navigation_history_command(&context))
        .await
    {
        Ok(DevToolsCommandResult::GetNavigationHistory(history)) => history,
        Ok(_) => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::UnknownError,
                "navigation history returned an unexpected result",
            ));
        }
        Err(error) => return classic_error_into_response(classic_error_from_devtools_error(error)),
    };
    let Some((entry_id, url)) = history_traversal_entry(&history, delta) else {
        return classic_success_into_response(Value::Null);
    };
    match webdriver_classic_execute_page_load_command(
        &binding,
        traverse_history_command(
            &context,
            entry_id,
            url,
            binding.page_load_strategy.navigation_wait(),
        ),
    )
    .await
    {
        Ok(DevToolsCommandResult::TraverseHistory(result)) => {
            if !result.same_document {
                classic_reset_to_top_level_browsing_context(&state, &session_id);
            }
            classic_success_into_response(Value::Null)
        }
        Ok(DevToolsCommandResult::Empty) => classic_success_into_response(Value::Null),
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "history traversal returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(error),
    }
}

async fn webdriver_classic_execute_page_load_command(
    binding: &ClassicSessionBinding,
    command: DevToolsCommand,
) -> Result<DevToolsCommandResult, ClassicError> {
    match binding
        .runtime
        .execute_inner(
            command,
            binding.timeouts.page_load.map(Duration::from_millis),
        )
        .await
    {
        Ok(result) => Ok(result),
        Err(error) if error.kind == DevToolsErrorKind::Timeout => Err(ClassicError::new(
            ClassicErrorCode::Timeout,
            "page load timed out",
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_execute_sync(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let mut command = match execute_sync_command(&context, &params) {
        Ok(command) => command,
        Err(error) => return classic_error_into_response(error),
    };
    let script_argument_handles = match prepare_classic_script_argument_handles(
        &state,
        &binding,
        &context,
        &params,
        "execute sync script argument",
    )
    .await
    {
        Ok(handles) => handles,
        Err(error) => return classic_error_into_response(error),
    };
    apply_classic_script_argument_handles(&mut command, &script_argument_handles);
    let result = execute_classic_script_with_argument_page(
        &binding,
        command,
        binding.timeouts.script.map(Duration::from_millis),
        script_argument_handles.page_residence.clone(),
    )
    .await;
    release_classic_remote_objects(
        &binding,
        &context,
        script_argument_handles.remote_handles,
        "execute sync script argument",
    )
    .await;
    match result {
        Ok((DevToolsCommandResult::Script(result), page_residence)) => match *result {
            DevToolsScriptResult::Value(value) => {
                match classic_webdriver_script_result_value(
                    &state,
                    &binding,
                    &page_residence,
                    value.value,
                )
                .await
                {
                    Ok(value) => classic_success_into_response(value),
                    Err(error) => classic_error_into_response(error),
                }
            }
            DevToolsScriptResult::Exception(exception) => {
                classic_error_into_response(classic_execute_script_exception_error(exception))
            }
        },
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "execute sync returned an unexpected result",
        )),
        Err(error)
            if error.kind == DevToolsErrorKind::Timeout
                || error.message == "MissingDevToolsCommandResult" =>
        {
            classic_error_into_response(ClassicError::new(
                ClassicErrorCode::ScriptTimeout,
                error.message,
            ))
        }
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_execute_async(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let mut command = match execute_async_command(&context, &params) {
        Ok(command) => command,
        Err(error) => return classic_error_into_response(error),
    };
    let script_argument_handles = match prepare_classic_script_argument_handles(
        &state,
        &binding,
        &context,
        &params,
        "execute async script argument",
    )
    .await
    {
        Ok(handles) => handles,
        Err(error) => return classic_error_into_response(error),
    };
    apply_classic_script_argument_handles(&mut command, &script_argument_handles);
    let result = execute_classic_script_with_argument_page(
        &binding,
        command,
        binding.timeouts.script.map(Duration::from_millis),
        script_argument_handles.page_residence.clone(),
    )
    .await;
    release_classic_remote_objects(
        &binding,
        &context,
        script_argument_handles.remote_handles,
        "execute async script argument",
    )
    .await;
    match result {
        Ok((DevToolsCommandResult::Script(result), page_residence)) => match *result {
            DevToolsScriptResult::Value(value) => {
                match classic_webdriver_script_result_value(
                    &state,
                    &binding,
                    &page_residence,
                    value.value,
                )
                .await
                {
                    Ok(value) => classic_success_into_response(value),
                    Err(error) => classic_error_into_response(error),
                }
            }
            DevToolsScriptResult::Exception(exception) => {
                classic_error_into_response(classic_execute_script_exception_error(exception))
            }
        },
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "execute async returned an unexpected result",
        )),
        Err(error)
            if error.kind == DevToolsErrorKind::Timeout
                || error.message == "MissingDevToolsCommandResult" =>
        {
            classic_error_into_response(ClassicError::new(
                ClassicErrorCode::ScriptTimeout,
                error.message,
            ))
        }
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_find_element(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    webdriver_classic_find_element_with_mode(session_id, state, body, false).await
}

pub(super) async fn webdriver_classic_find_elements(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    webdriver_classic_find_element_with_mode(session_id, state, body, true).await
}

pub(super) async fn webdriver_classic_find_child_element(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    webdriver_classic_find_element_with_mode_and_root(
        session_id,
        state,
        body,
        false,
        Some(element_id),
    )
    .await
}

pub(super) async fn webdriver_classic_find_child_elements(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    webdriver_classic_find_element_with_mode_and_root(
        session_id,
        state,
        body,
        true,
        Some(element_id),
    )
    .await
}

pub(super) async fn webdriver_classic_get_element_shadow_root(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let reference = match resolve_classic_element_dom_reference(&state, &binding, &element_id) {
        Ok(reference) => reference,
        Err(error) => return classic_error_into_response(error),
    };

    if let Err(error) =
        verify_classic_element_attached(&state, &binding, &context, &element_id).await
    {
        return classic_error_into_response(error);
    }

    let page_residence = reference.page_residence.clone();
    let result = binding
        .runtime
        .execute_on_page(
            describe_node_reference_command(&context, reference.reference, 1, true),
            page_residence.clone(),
        )
        .await;
    match result {
        Ok(DevToolsCommandResult::DescribeNode(result)) => {
            let Some((node_id, reference)) =
                classic_author_shadow_root_reference_from_described_node(&result.node)
            else {
                return classic_error_into_response(ClassicError::new(
                    ClassicErrorCode::NoSuchShadowRoot,
                    "shadow root not found",
                ));
            };
            classic_success_into_response(
                registered_classic_shadow_root_reference_from_dom_reference(
                    &state,
                    &binding,
                    &page_residence,
                    node_id,
                    reference,
                ),
            )
        }
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "get element shadow root returned an unexpected result",
        )),
        Err(error) if error.kind == DevToolsErrorKind::NoSuchNode => {
            classic_error_into_response(ClassicError::new(
                ClassicErrorCode::StaleElementReference,
                "element is no longer attached to the DOM",
            ))
        }
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

async fn verify_classic_element_attached(
    state: &AppState,
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
) -> Result<(), ClassicError> {
    let object_id = resolve_classic_remote_object(
        state,
        binding,
        context,
        element_id,
        "webdriver-classic-shadow-host",
        "get element shadow root",
    )
    .await?;
    let result = binding
        .runtime
        .execute_on_page(
            verify_element_attached_command(context, object_id.object_id.clone()),
            object_id.page_residence.clone(),
        )
        .await;
    release_classic_page_bound_remote_object(
        binding,
        context,
        object_id,
        "get element shadow root",
    )
    .await;
    match result {
        Ok(DevToolsCommandResult::Script(result)) => match *result {
            DevToolsScriptResult::Value(_) => Ok(()),
            DevToolsScriptResult::Exception(exception) => {
                Err(classic_webdriver_command_exception_error(exception))
            }
        },
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "get element shadow root attachment check returned an unexpected result",
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

fn classic_author_shadow_root_reference_from_described_node(
    node: &Value,
) -> Option<(u32, DevToolsDomNodeReference)> {
    let shadow_root = node
        .get("shadowRoots")?
        .as_array()?
        .iter()
        .find(|shadow_root| {
            matches!(
                shadow_root.get("shadowRootType").and_then(Value::as_str),
                Some("open" | "closed")
            )
        })?;
    if let Some(backend_node_id) = shadow_root
        .get("backendNodeId")
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .filter(|id| *id > 0)
    {
        return Some((
            backend_node_id,
            DevToolsDomNodeReference::BackendNodeId(backend_node_id),
        ));
    }
    let node_id = shadow_root
        .get("nodeId")?
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .filter(|id| *id > 0)?;
    Some((node_id, DevToolsDomNodeReference::FrontendNodeId(node_id)))
}

pub(super) async fn webdriver_classic_find_shadow_element(
    Path((session_id, shadow_root_id)): Path<(String, String)>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    webdriver_classic_find_shadow_element_with_mode(session_id, shadow_root_id, state, body, false)
        .await
}

pub(super) async fn webdriver_classic_find_shadow_elements(
    Path((session_id, shadow_root_id)): Path<(String, String)>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    webdriver_classic_find_shadow_element_with_mode(session_id, shadow_root_id, state, body, true)
        .await
}

pub(super) async fn webdriver_classic_find_element_with_mode(
    session_id: String,
    state: AppState,
    body: Bytes,
    multiple: bool,
) -> Response {
    webdriver_classic_find_element_with_mode_and_root(session_id, state, body, multiple, None).await
}

async fn webdriver_classic_find_shadow_element_with_mode(
    session_id: String,
    shadow_root_id: String,
    state: AppState,
    body: Bytes,
    multiple: bool,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let shadow_root_reference =
        match resolve_classic_shadow_root_dom_reference(&state, &binding, &shadow_root_id) {
            Ok(reference) => reference,
            Err(error) => return classic_error_into_response(error),
        };
    if let Err(error) =
        verify_classic_shadow_root_attached(&binding, &context, shadow_root_reference.clone()).await
    {
        return classic_error_into_response(error);
    }
    let expected_page = shadow_root_reference.page_residence.clone();
    let root = Some(shadow_root_reference.reference);
    let command = match find_element_command_with_root(&context, &params, multiple, root) {
        Ok(command) => command,
        Err(error) => return classic_error_into_response(error),
    };
    match webdriver_classic_execute_find_element_command(
        &binding,
        command,
        multiple,
        Some(expected_page),
    )
    .await
    {
        Ok(result) => {
            webdriver_classic_find_element_result_response(
                &state,
                &binding,
                result,
                multiple,
                "find shadow-root element",
            )
            .await
        }
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

async fn webdriver_classic_find_element_with_mode_and_root(
    session_id: String,
    state: AppState,
    body: Bytes,
    multiple: bool,
    root_element_id: Option<String>,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let (command, expected_page) = if let Some(root_element_id) = root_element_id {
        let root = match resolve_classic_element_dom_reference(&state, &binding, &root_element_id) {
            Ok(reference) => reference,
            Err(error) => return classic_error_into_response(error),
        };
        let expected_page = root.page_residence.clone();
        match find_element_command_with_root(&context, &params, multiple, Some(root.reference)) {
            Ok(command) => (command, Some(expected_page)),
            Err(error) => return classic_error_into_response(error),
        }
    } else {
        match find_element_command(&context, &params, multiple) {
            Ok(command) => (command, None),
            Err(error) => return classic_error_into_response(error),
        }
    };
    match webdriver_classic_execute_find_element_command(&binding, command, multiple, expected_page)
        .await
    {
        Ok(result) => {
            webdriver_classic_find_element_result_response(
                &state,
                &binding,
                result,
                multiple,
                "find element",
            )
            .await
        }
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

async fn verify_classic_shadow_root_attached(
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    shadow_root_reference: ClassicPageBoundDomReference,
) -> Result<(), ClassicError> {
    let object_id = resolve_classic_shadow_root_remote_object_from_reference(
        binding,
        context,
        shadow_root_reference,
        None,
        "webdriver-classic-shadow-root",
        "find shadow-root element",
    )
    .await?;
    let result = binding
        .runtime
        .execute_on_page(
            shadow_root_attached_command(context, object_id.object_id.clone()),
            object_id.page_residence.clone(),
        )
        .await;
    release_classic_page_bound_remote_object(
        binding,
        context,
        object_id,
        "find shadow-root element",
    )
    .await;
    verify_classic_shadow_root_attached_result(result, "find shadow-root element")
}

async fn verify_classic_shadow_root_remote_object_attached(
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    remote_object: &ClassicPageBoundRemoteObject,
    label: &str,
) -> Result<(), ClassicError> {
    let result = binding
        .runtime
        .execute_on_page(
            shadow_root_attached_command(context, remote_object.object_id.clone()),
            remote_object.page_residence.clone(),
        )
        .await;
    verify_classic_shadow_root_attached_result(result, label)
}

fn verify_classic_shadow_root_attached_result(
    result: Result<DevToolsCommandResult, DevToolsError>,
    label: &str,
) -> Result<(), ClassicError> {
    match result {
        Ok(DevToolsCommandResult::Script(result)) => match *result {
            DevToolsScriptResult::Value(value) if value.value.as_bool().unwrap_or(false) => Ok(()),
            DevToolsScriptResult::Value(_) | DevToolsScriptResult::Exception(_) => {
                Err(ClassicError::new(
                    ClassicErrorCode::DetachedShadowRoot,
                    "shadow root is detached",
                ))
            }
        },
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            format!("{label} attachment check returned an unexpected result"),
        )),
        Err(_) => Err(ClassicError::new(
            ClassicErrorCode::DetachedShadowRoot,
            "shadow root is detached",
        )),
    }
}

async fn webdriver_classic_execute_find_element_command(
    binding: &ClassicSessionBinding,
    command: DevToolsCommand,
    multiple: bool,
    expected_page: Option<DevToolsPageResidenceIdentity>,
) -> Result<ClassicFindElementExecution, DevToolsError> {
    let implicit_wait = Duration::from_millis(binding.timeouts.implicit.unwrap_or(0));
    let started = tokio::time::Instant::now();
    loop {
        let execution = match expected_page.as_ref() {
            Some(expected_page) => binding
                .runtime
                .execute_on_page(command.clone(), expected_page.clone())
                .await
                .map(|result| (result, expected_page.clone())),
            None => {
                binding
                    .runtime
                    .execute_with_page_residence(command.clone())
                    .await
            }
        };
        match execution {
            Ok((DevToolsCommandResult::QuerySelector(result), page_residence)) => {
                if !result.node_ids.is_empty()
                    || implicit_wait.is_zero()
                    || started.elapsed() >= implicit_wait
                {
                    return Ok(ClassicFindElementExecution {
                        result: DevToolsCommandResult::QuerySelector(result),
                        page_residence: Some(page_residence),
                    });
                }
            }
            Ok((DevToolsCommandResult::LocateNodes(result), page_residence)) => {
                if !result.node_ids.is_empty()
                    || implicit_wait.is_zero()
                    || started.elapsed() >= implicit_wait
                {
                    return Ok(ClassicFindElementExecution {
                        result: DevToolsCommandResult::LocateNodes(result),
                        page_residence: Some(page_residence),
                    });
                }
            }
            Ok((result, page_residence)) => {
                return Ok(ClassicFindElementExecution {
                    result,
                    page_residence: Some(page_residence),
                });
            }
            Err(error)
                if classic_find_element_should_retry_error(&error)
                    && !implicit_wait.is_zero()
                    && started.elapsed() < implicit_wait => {}
            Err(error) => return Err(error),
        }

        let remaining = implicit_wait.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Ok(ClassicFindElementExecution {
                result: empty_classic_find_result_for_command(&command, multiple),
                page_residence: None,
            });
        }
        sleep(remaining.min(CLASSIC_IMPLICIT_WAIT_POLL_INTERVAL)).await;
    }
}

struct ClassicFindElementExecution {
    result: DevToolsCommandResult,
    page_residence: Option<DevToolsPageResidenceIdentity>,
}

async fn webdriver_classic_find_element_result_response(
    state: &AppState,
    binding: &ClassicSessionBinding,
    execution: ClassicFindElementExecution,
    multiple: bool,
    label: &str,
) -> Response {
    let ClassicFindElementExecution {
        result,
        page_residence,
    } = execution;
    match result {
        DevToolsCommandResult::QuerySelector(result) if multiple => {
            if result.node_ids.is_empty() {
                return classic_success_into_response(json!([]));
            }
            let page_residence = page_residence
                .as_ref()
                .expect("non-empty find result must retain its Page residence");
            let references = match classic_find_canonical_dom_references_by_frontend_node_id(
                binding,
                page_residence,
                result.node_ids,
                label,
            )
            .await
            {
                Ok(references) => references,
                Err(error) => return classic_error_into_response(error),
            };
            classic_success_into_response(json!(
                registered_classic_element_references_from_canonical_references(
                    state,
                    binding,
                    page_residence,
                    references
                )
            ))
        }
        DevToolsCommandResult::LocateNodes(result) if multiple => {
            if result.node_ids.is_empty() {
                return classic_success_into_response(json!([]));
            }
            let page_residence = page_residence
                .as_ref()
                .expect("non-empty find result must retain its Page residence");
            let references = match classic_find_canonical_dom_references_by_frontend_node_id(
                binding,
                page_residence,
                result.node_ids,
                label,
            )
            .await
            {
                Ok(references) => references,
                Err(error) => return classic_error_into_response(error),
            };
            classic_success_into_response(json!(
                registered_classic_element_references_from_canonical_references(
                    state,
                    binding,
                    page_residence,
                    references
                )
            ))
        }
        DevToolsCommandResult::QuerySelector(result) => {
            let Some(frontend_node_id) = result.node_ids.first().copied() else {
                return classic_error_into_response(ClassicError::new(
                    ClassicErrorCode::NoSuchElement,
                    "element not found",
                ));
            };
            let reference = match classic_find_canonical_dom_reference_for_frontend_node_id(
                binding,
                page_residence
                    .as_ref()
                    .expect("non-empty find result must retain its Page residence"),
                frontend_node_id,
                label,
            )
            .await
            {
                Ok(reference) => reference,
                Err(error) => return classic_error_into_response(error),
            };
            classic_success_into_response(registered_classic_element_reference_from_dom_reference(
                state,
                binding,
                page_residence
                    .as_ref()
                    .expect("non-empty find result must retain its Page residence"),
                reference.node_id,
                reference.reference,
            ))
        }
        DevToolsCommandResult::LocateNodes(result) => {
            let Some(frontend_node_id) = result.node_ids.first().copied() else {
                return classic_error_into_response(ClassicError::new(
                    ClassicErrorCode::NoSuchElement,
                    "element not found",
                ));
            };
            let reference = match classic_find_canonical_dom_reference_for_frontend_node_id(
                binding,
                page_residence
                    .as_ref()
                    .expect("non-empty find result must retain its Page residence"),
                frontend_node_id,
                label,
            )
            .await
            {
                Ok(reference) => reference,
                Err(error) => return classic_error_into_response(error),
            };
            classic_success_into_response(registered_classic_element_reference_from_dom_reference(
                state,
                binding,
                page_residence
                    .as_ref()
                    .expect("non-empty find result must retain its Page residence"),
                reference.node_id,
                reference.reference,
            ))
        }
        _ => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            format!("{label} returned an unexpected result"),
        )),
    }
}

async fn classic_find_canonical_dom_references_by_frontend_node_id(
    binding: &ClassicSessionBinding,
    page_residence: &DevToolsPageResidenceIdentity,
    frontend_node_ids: Vec<u32>,
    label: &str,
) -> Result<Vec<ClassicScriptCanonicalNodeReference>, ClassicError> {
    let mut cache: BTreeMap<u32, ClassicScriptCanonicalNodeReference> = BTreeMap::new();
    let mut out = Vec::with_capacity(frontend_node_ids.len());
    for frontend_node_id in frontend_node_ids {
        let reference = if let Some(reference) = cache.get(&frontend_node_id) {
            reference.clone()
        } else {
            let reference = classic_find_canonical_dom_reference_for_frontend_node_id(
                binding,
                page_residence,
                frontend_node_id,
                label,
            )
            .await?;
            cache.insert(frontend_node_id, reference.clone());
            reference
        };
        out.push(reference);
    }
    Ok(out)
}

async fn classic_find_canonical_dom_reference_for_frontend_node_id(
    binding: &ClassicSessionBinding,
    page_residence: &DevToolsPageResidenceIdentity,
    frontend_node_id: u32,
    label: &str,
) -> Result<ClassicScriptCanonicalNodeReference, ClassicError> {
    let context = classic_browsing_context(binding);
    match binding
        .runtime
        .execute_on_page(
            describe_node_command(&context, frontend_node_id, 0, false),
            page_residence.clone(),
        )
        .await
    {
        Ok(DevToolsCommandResult::DescribeNode(result)) => {
            Ok(classic_script_canonical_dom_reference_from_described_node(
                &result.node,
                frontend_node_id,
            ))
        }
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            format!("{label} node lookup returned an unexpected result"),
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

fn empty_classic_find_result_for_command(
    command: &DevToolsCommand,
    multiple: bool,
) -> DevToolsCommandResult {
    match command {
        DevToolsCommand::LocateNodes(_) => {
            DevToolsCommandResult::LocateNodes(DevToolsLocateNodesResult {
                nodes: Vec::new(),
                node_ids: Vec::new(),
            })
        }
        _ => DevToolsCommandResult::QuerySelector(DevToolsQuerySelectorResult {
            node_ids: Vec::new(),
            multiple,
        }),
    }
}

fn classic_find_element_should_retry_error(error: &DevToolsError) -> bool {
    matches!(error.kind, DevToolsErrorKind::NavigationChangingDocument)
}

pub(super) async fn webdriver_classic_get_element_attribute(
    Path((session_id, element_id, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let reference = match resolve_classic_element_dom_reference(&state, &binding, &element_id) {
        Ok(reference) => reference,
        Err(error) => return classic_error_into_response(error),
    };
    let command = get_element_attributes_reference_command(&context, reference.reference);
    match binding
        .runtime
        .execute_on_page(command, reference.page_residence)
        .await
    {
        Ok(DevToolsCommandResult::GetAttributes(result)) => {
            classic_success_into_response(json!(classic_attribute_value(result, &name)))
        }
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "get element attribute returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_get_element_text(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let object_id = match resolve_classic_remote_object(
        &state,
        &binding,
        &context,
        &element_id,
        "webdriver-classic-text",
        "get element text",
    )
    .await
    {
        Ok(object_id) => object_id,
        Err(error) if classic_text_should_fallback_to_dom_text(&error) => {
            return webdriver_classic_get_element_text_from_dom(
                &state,
                &binding,
                &context,
                &element_id,
            )
            .await;
        }
        Err(error) => return classic_error_into_response(error),
    };
    let result = binding
        .runtime
        .execute_on_page(
            get_element_rendered_text_command(&context, object_id.object_id.clone()),
            object_id.page_residence.clone(),
        )
        .await;
    release_classic_page_bound_remote_object(&binding, &context, object_id, "get element text")
        .await;
    match result {
        Ok(DevToolsCommandResult::Script(result)) => match *result {
            DevToolsScriptResult::Value(value) => {
                classic_success_into_response(json!(value.value.as_str().unwrap_or_default()))
            }
            DevToolsScriptResult::Exception(exception) => {
                classic_error_into_response(classic_webdriver_command_exception_error(exception))
            }
        },
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "get element text returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

async fn webdriver_classic_get_element_text_from_dom(
    state: &AppState,
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
) -> Response {
    let reference = match resolve_classic_element_dom_reference(state, binding, element_id) {
        Ok(reference) => reference,
        Err(error) => return classic_error_into_response(error),
    };
    let command = get_element_text_reference_command(context, reference.reference);
    match binding
        .runtime
        .execute_on_page(command, reference.page_residence)
        .await
    {
        Ok(DevToolsCommandResult::GetText(result)) => {
            classic_success_into_response(json!(classic_text_value(result)))
        }
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "get element text returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

fn classic_text_should_fallback_to_dom_text(error: &ClassicError) -> bool {
    matches!(
        error.code,
        ClassicErrorCode::NoSuchElement | ClassicErrorCode::StaleElementReference
    )
}

pub(super) async fn webdriver_classic_get_element_tag_name(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let reference = match resolve_classic_element_dom_reference(&state, &binding, &element_id) {
        Ok(reference) => reference,
        Err(error) => return classic_error_into_response(error),
    };
    let command =
        get_element_property_reference_command(&context, reference.reference, "localName");
    match binding
        .runtime
        .execute_on_page(command, reference.page_residence)
        .await
    {
        Ok(DevToolsCommandResult::GetProperty(result)) => {
            classic_success_into_response(classic_property_value(result))
        }
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "get element tag name returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_get_active_element(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    match binding
        .runtime
        .execute_with_page_residence(active_element_command(&context))
        .await
    {
        Ok((DevToolsCommandResult::Script(result), page_residence)) => match *result {
            DevToolsScriptResult::Value(value) => {
                let Some(node_id) = value.node_id else {
                    return classic_error_into_response(ClassicError::new(
                        ClassicErrorCode::UnknownError,
                        "active element returned a non-node value",
                    ));
                };
                let reference = value
                    .backend_node_id
                    .map(DevToolsDomNodeReference::BackendNodeId)
                    .unwrap_or(DevToolsDomNodeReference::FrontendNodeId(node_id));
                let owner_node_id = value.backend_node_id.unwrap_or(node_id);
                release_classic_remote_handle(
                    &binding,
                    &context,
                    &value,
                    &page_residence,
                    "active element",
                )
                .await;
                classic_success_into_response(
                    registered_classic_element_reference_from_dom_reference(
                        &state,
                        &binding,
                        &page_residence,
                        owner_node_id,
                        reference,
                    ),
                )
            }
            DevToolsScriptResult::Exception(exception) => {
                classic_error_into_response(classic_webdriver_command_exception_error(exception))
            }
        },
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "active element returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_element_equals(
    Path((session_id, element_id, other_element_id)): Path<(String, String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let element_identity =
        match classic_element_live_identity(&state, &binding, &context, &element_id).await {
            Ok(identity) => identity,
            Err(error) => return classic_error_into_response(error),
        };
    let other_element_identity =
        match classic_element_live_identity(&state, &binding, &context, &other_element_id).await {
            Ok(identity) => identity,
            Err(error) => return classic_error_into_response(error),
        };
    classic_success_into_response(json!(element_identity == other_element_identity))
}

async fn classic_element_live_identity(
    state: &AppState,
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
) -> Result<ClassicElementLiveIdentity, ClassicError> {
    let reference = resolve_classic_element_dom_reference(state, binding, element_id)?;
    let command = describe_node_reference_command(context, reference.reference, 0, false);
    match binding
        .runtime
        .execute_on_page(command, reference.page_residence)
        .await
    {
        Ok(DevToolsCommandResult::DescribeNode(result)) => {
            classic_element_live_identity_from_described_node(&result.node).ok_or_else(|| {
                ClassicError::new(
                    ClassicErrorCode::NoSuchElement,
                    "element no longer describes a live DOM node",
                )
            })
        }
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "element equality returned an unexpected result",
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

fn classic_element_live_identity_from_described_node(
    node: &Value,
) -> Option<ClassicElementLiveIdentity> {
    node.get("backendNodeId")
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .filter(|id| *id > 0)
        .map(ClassicElementLiveIdentity::BackendNodeId)
        .or_else(|| {
            node.get("nodeId")
                .and_then(Value::as_u64)
                .and_then(|id| u32::try_from(id).ok())
                .filter(|id| *id > 0)
                .map(ClassicElementLiveIdentity::FrontendNodeId)
        })
}

pub(super) async fn webdriver_classic_is_element_enabled(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let object_id = match resolve_classic_remote_object(
        &state,
        &binding,
        &context,
        &element_id,
        "webdriver-classic-enabled",
        "is element enabled",
    )
    .await
    {
        Ok(object_id) => object_id,
        Err(error) => return classic_error_into_response(error),
    };
    let result = binding
        .runtime
        .execute_on_page(
            get_element_enabled_command(&context, object_id.object_id.clone()),
            object_id.page_residence.clone(),
        )
        .await;
    release_classic_page_bound_remote_object(&binding, &context, object_id, "is element enabled")
        .await;
    match result {
        Ok(DevToolsCommandResult::Script(result)) => match *result {
            DevToolsScriptResult::Value(value) => {
                classic_success_into_response(json!(value.value.as_bool().unwrap_or(false)))
            }
            DevToolsScriptResult::Exception(exception) => {
                classic_error_into_response(classic_webdriver_command_exception_error(exception))
            }
        },
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "is element enabled returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_is_element_selected(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let tag_name = match webdriver_classic_element_property_value(
        &state,
        &binding,
        &context,
        &element_id,
        "localName",
        "is element selected",
    )
    .await
    {
        Ok(value) => value.as_str().unwrap_or_default().to_owned(),
        Err(error) => return classic_error_into_response(error),
    };
    let selected = match tag_name.as_str() {
        "option" => match webdriver_classic_element_property_value(
            &state,
            &binding,
            &context,
            &element_id,
            "selected",
            "is element selected",
        )
        .await
        {
            Ok(value) => value.as_bool().unwrap_or(false),
            Err(error) => return classic_error_into_response(error),
        },
        "input" => {
            let input_type = match webdriver_classic_element_property_value(
                &state,
                &binding,
                &context,
                &element_id,
                "type",
                "is element selected",
            )
            .await
            {
                Ok(value) => value.as_str().unwrap_or_default().to_ascii_lowercase(),
                Err(error) => return classic_error_into_response(error),
            };
            if input_type == "checkbox" || input_type == "radio" {
                match webdriver_classic_element_property_value(
                    &state,
                    &binding,
                    &context,
                    &element_id,
                    "checked",
                    "is element selected",
                )
                .await
                {
                    Ok(value) => value.as_bool().unwrap_or(false),
                    Err(error) => return classic_error_into_response(error),
                }
            } else {
                false
            }
        }
        _ => false,
    };
    classic_success_into_response(json!(selected))
}

pub(super) async fn webdriver_classic_get_element_rect(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let reference = match resolve_classic_element_dom_reference(&state, &binding, &element_id) {
        Ok(reference) => reference,
        Err(error) => return classic_error_into_response(error),
    };
    let command = get_element_rect_reference_command(&context, reference.reference);
    match binding
        .runtime
        .execute_on_page(command, reference.page_residence)
        .await
    {
        Ok(DevToolsCommandResult::DomGeometry(result)) => match classic_rect_from_geometry(&result)
        {
            Ok(rect) => classic_success_into_response(rect),
            Err(error) => classic_error_into_response(error),
        },
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "get element rect returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_take_element_screenshot(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let object_id = match resolve_classic_remote_object(
        &state,
        &binding,
        &context,
        &element_id,
        "webdriver-classic-element-screenshot",
        "element screenshot",
    )
    .await
    {
        Ok(object_id) => object_id,
        Err(error) => return classic_error_into_response(error),
    };
    let result = binding
        .runtime
        .execute_on_page(
            element_screenshot_command(&context, object_id.object_id.clone()),
            object_id.page_residence.clone(),
        )
        .await;
    release_classic_page_bound_remote_object(&binding, &context, object_id, "element screenshot")
        .await;
    match result {
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "element screenshot returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_get_element_css_value(
    Path((session_id, element_id, property_name)): Path<(String, String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let object_id = match resolve_classic_remote_object(
        &state,
        &binding,
        &context,
        &element_id,
        "webdriver-classic-css",
        "get element CSS value",
    )
    .await
    {
        Ok(object_id) => object_id,
        Err(error) => return classic_error_into_response(error),
    };
    let command =
        get_element_css_value_command(&context, object_id.object_id.clone(), property_name);
    let result = binding
        .runtime
        .execute_on_page(command, object_id.page_residence.clone())
        .await;
    release_classic_page_bound_remote_object(
        &binding,
        &context,
        object_id,
        "get element CSS value",
    )
    .await;
    match result {
        Ok(DevToolsCommandResult::Script(result)) => match *result {
            DevToolsScriptResult::Value(value) => {
                classic_success_into_response(json!(value.value.as_str().unwrap_or_default()))
            }
            DevToolsScriptResult::Exception(exception) => {
                classic_error_into_response(classic_webdriver_command_exception_error(exception))
            }
        },
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "get element CSS value returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_get_element_computed_label(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    webdriver_classic_get_element_computed_value(
        state,
        session_id,
        element_id,
        ClassicComputedElementValue::Label,
    )
    .await
}

pub(super) async fn webdriver_classic_get_element_computed_role(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    webdriver_classic_get_element_computed_value(
        state,
        session_id,
        element_id,
        ClassicComputedElementValue::Role,
    )
    .await
}

#[derive(Clone, Copy)]
enum ClassicComputedElementValue {
    Label,
    Role,
}

impl ClassicComputedElementValue {
    fn label(self) -> &'static str {
        match self {
            Self::Label => "get element computed label",
            Self::Role => "get element computed role",
        }
    }

    fn object_group(self) -> &'static str {
        match self {
            Self::Label => "webdriver-classic-computed-label",
            Self::Role => "webdriver-classic-computed-role",
        }
    }

    fn command(
        self,
        context: &ClassicDevToolsCommandContext,
        object_id: String,
    ) -> DevToolsCommand {
        match self {
            Self::Label => get_element_computed_label_command(context, object_id),
            Self::Role => get_element_computed_role_command(context, object_id),
        }
    }
}

async fn webdriver_classic_get_element_computed_value(
    state: AppState,
    session_id: String,
    element_id: String,
    kind: ClassicComputedElementValue,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let object_id = match resolve_classic_remote_object(
        &state,
        &binding,
        &context,
        &element_id,
        kind.object_group(),
        kind.label(),
    )
    .await
    {
        Ok(object_id) => object_id,
        Err(error) => return classic_error_into_response(error),
    };
    let result = binding
        .runtime
        .execute_on_page(
            kind.command(&context, object_id.object_id.clone()),
            object_id.page_residence.clone(),
        )
        .await;
    release_classic_page_bound_remote_object(&binding, &context, object_id, kind.label()).await;
    match result {
        Ok(DevToolsCommandResult::Script(result)) => match *result {
            DevToolsScriptResult::Value(value) => {
                classic_success_into_response(json!(value.value.as_str().unwrap_or_default()))
            }
            DevToolsScriptResult::Exception(exception) => {
                classic_error_into_response(classic_execute_script_exception_error(exception))
            }
        },
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            format!("{} returned an unexpected result", kind.label()),
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_is_element_displayed(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let object_id = match resolve_classic_remote_object(
        &state,
        &binding,
        &context,
        &element_id,
        "webdriver-classic-displayed",
        "is element displayed",
    )
    .await
    {
        Ok(object_id) => object_id,
        Err(error) => return classic_error_into_response(error),
    };
    let command = get_element_displayed_command(&context, object_id.object_id.clone());
    let result = binding
        .runtime
        .execute_on_page(command, object_id.page_residence.clone())
        .await;
    release_classic_page_bound_remote_object(&binding, &context, object_id, "is element displayed")
        .await;
    match result {
        Ok(DevToolsCommandResult::Script(result)) => match *result {
            DevToolsScriptResult::Value(value) => {
                classic_success_into_response(json!(value.value.as_bool().unwrap_or(false)))
            }
            DevToolsScriptResult::Exception(exception) => {
                classic_error_into_response(classic_webdriver_command_exception_error(exception))
            }
        },
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "is element displayed returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

async fn webdriver_classic_element_property_value(
    state: &AppState,
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
    name: impl Into<String>,
    label: &'static str,
) -> Result<Value, ClassicError> {
    let reference = resolve_classic_element_dom_reference(state, binding, element_id)?;
    let command = get_element_property_reference_command(context, reference.reference, name);
    match binding
        .runtime
        .execute_on_page(command, reference.page_residence)
        .await
    {
        Ok(DevToolsCommandResult::GetProperty(result)) => Ok(classic_property_value(result)),
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            format!("{label} returned an unexpected result"),
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_get_element_property(
    Path((session_id, element_id, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let params = json!({
        "script": "return arguments[0][arguments[1]];",
        "args": [
            { CLASSIC_ELEMENT_REFERENCE_KEY: element_id },
            name
        ],
    });
    let mut command = match execute_sync_command(&context, &params) {
        Ok(command) => command,
        Err(error) => return classic_error_into_response(error),
    };
    let script_argument_handles = match prepare_classic_script_argument_handles(
        &state,
        &binding,
        &context,
        &params,
        "get element property argument",
    )
    .await
    {
        Ok(handles) => handles,
        Err(error) => return classic_error_into_response(error),
    };
    apply_classic_script_argument_handles(&mut command, &script_argument_handles);
    let result = execute_classic_script_with_argument_page(
        &binding,
        command,
        binding.timeouts.script.map(Duration::from_millis),
        script_argument_handles.page_residence.clone(),
    )
    .await;
    release_classic_remote_objects(
        &binding,
        &context,
        script_argument_handles.remote_handles,
        "get element property argument",
    )
    .await;
    match result {
        Ok((DevToolsCommandResult::Script(result), page_residence)) => match *result {
            DevToolsScriptResult::Value(value) => {
                match classic_webdriver_script_result_value(
                    &state,
                    &binding,
                    &page_residence,
                    value.value,
                )
                .await
                {
                    Ok(value) => classic_success_into_response(value),
                    Err(error) => classic_error_into_response(error),
                }
            }
            DevToolsScriptResult::Exception(exception) => {
                classic_error_into_response(classic_execute_script_exception_error(exception))
            }
        },
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "get element property returned an unexpected result",
        )),
        Err(error)
            if error.kind == DevToolsErrorKind::Timeout
                || error.message == "MissingDevToolsCommandResult" =>
        {
            classic_error_into_response(ClassicError::new(
                ClassicErrorCode::ScriptTimeout,
                error.message,
            ))
        }
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_clear_element(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let object_id = match resolve_classic_remote_object(
        &state,
        &binding,
        &context,
        &element_id,
        "webdriver-classic-clear",
        "clear element",
    )
    .await
    {
        Ok(object_id) => object_id,
        Err(error) => return classic_error_into_response(error),
    };

    let result = binding
        .runtime
        .execute_on_page(
            clear_element_command(&context, object_id.object_id.clone()),
            object_id.page_residence.clone(),
        )
        .await;
    release_classic_page_bound_remote_object(&binding, &context, object_id, "clear element").await;
    match result {
        Ok(DevToolsCommandResult::Script(result)) => match *result {
            DevToolsScriptResult::Value(value) => match classic_clear_element_result(value.value) {
                Ok(()) => classic_success_into_response(Value::Null),
                Err(error) => classic_error_into_response(error),
            },
            DevToolsScriptResult::Exception(exception) => {
                classic_error_into_response(classic_webdriver_command_exception_error(exception))
            }
        },
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "clear element returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

fn classic_clear_element_result(value: Value) -> Result<(), ClassicError> {
    match value.get("status").and_then(Value::as_str) {
        Some("success") => Ok(()),
        Some("invalid element state") => Err(ClassicError::new(
            ClassicErrorCode::InvalidElementState,
            "element cannot be cleared",
        )),
        Some("unsupported") => Err(ClassicError::new(
            ClassicErrorCode::InvalidElementState,
            "element is not clearable",
        )),
        _ => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "clear element returned an invalid result",
        )),
    }
}

pub(super) async fn webdriver_classic_click_element(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    if let Err(error) = webdriver_classic_activate_element_by_handle(
        &state,
        &binding,
        &context,
        &element_id,
        "element click",
        "element click returned an unexpected result",
    )
    .await
    {
        return classic_error_into_response(error);
    }
    classic_success_into_response(Value::Null)
}

pub(super) async fn webdriver_classic_send_keys_to_element(
    Path((session_id, element_id)): Path<(String, String)>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let text = match element_send_keys_text(&params) {
        Ok(text) => text,
        Err(error) => return classic_error_into_response(error),
    };
    let binding = match classic_binding_with_existing_current_browsing_context(binding).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let is_file_input = match webdriver_classic_element_is_file_input(
        &state,
        &binding,
        &context,
        &element_id,
    )
    .await
    {
        Ok(is_file_input) => is_file_input,
        Err(error) => return classic_error_into_response(error),
    };
    if is_file_input {
        match webdriver_classic_send_file_input_keys(&state, &binding, &context, &element_id, &text)
            .await
        {
            Ok(()) => {
                return classic_success_into_response(Value::Null);
            }
            Err(error) => return classic_error_into_response(error),
        }
    }
    let geometry =
        match webdriver_classic_element_geometry(&state, &binding, &context, &element_id).await {
            Ok(geometry) => geometry,
            Err(error) => return classic_error_into_response(error),
        };
    let send_keys_preflight = match webdriver_classic_prepare_text_control_for_send_keys(
        &state,
        &binding,
        &context,
        &element_id,
        &text,
    )
    .await
    {
        Ok(preflight) => preflight,
        Err(error) => return classic_error_into_response(error),
    };
    if matches!(send_keys_preflight, ClassicSendKeysPreflight::ValueSet) {
        return classic_success_into_response(Value::Null);
    }
    if matches!(
        send_keys_preflight,
        ClassicSendKeysPreflight::NotTextControl
    ) {
        let input_context = classic_top_level_context(&binding);
        let commands = match element_click_input_commands(&input_context, &geometry) {
            Ok(commands) => commands,
            Err(error) => return classic_error_into_response(error),
        };
        if let Err(error) = webdriver_classic_execute_empty_commands(
            &binding,
            commands,
            "element send keys focus returned an unexpected result",
        )
        .await
        {
            return classic_error_into_response(error);
        }
    }
    if let Err(error) = webdriver_classic_execute_empty_commands(
        &binding,
        element_send_keys_input_commands(&classic_top_level_context(&binding), &text),
        "element send keys input returned an unexpected result",
    )
    .await
    {
        return classic_error_into_response(error);
    }
    classic_success_into_response(Value::Null)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClassicSendKeysPreflight {
    TextControl,
    ValueSet,
    NotTextControl,
}

async fn webdriver_classic_prepare_text_control_for_send_keys(
    state: &AppState,
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
    text: &str,
) -> Result<ClassicSendKeysPreflight, ClassicError> {
    let object_id = resolve_classic_remote_object(
        state,
        binding,
        context,
        element_id,
        "webdriver-classic-send-keys-preflight",
        "element send keys preflight",
    )
    .await?;
    let command =
        element_send_keys_prepare_text_control_command(context, object_id.object_id.clone(), text);
    let result = binding
        .runtime
        .execute_on_page(command, object_id.page_residence.clone())
        .await;
    release_classic_page_bound_remote_object(
        binding,
        context,
        object_id,
        "element send keys preflight",
    )
    .await;
    match result {
        Ok(DevToolsCommandResult::Script(result)) => match *result {
            DevToolsScriptResult::Value(value) => match value.value.as_str() {
                Some("text-control") => Ok(ClassicSendKeysPreflight::TextControl),
                Some("value-set") => Ok(ClassicSendKeysPreflight::ValueSet),
                Some("not-text-control") => Ok(ClassicSendKeysPreflight::NotTextControl),
                _ => Err(ClassicError::new(
                    ClassicErrorCode::UnknownError,
                    "element send keys preflight returned an unexpected value",
                )),
            },
            DevToolsScriptResult::Exception(exception) => {
                Err(classic_webdriver_command_exception_error(exception))
            }
        },
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "element send keys preflight returned an unexpected result",
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

async fn webdriver_classic_activate_element_by_handle(
    state: &AppState,
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
    object_group: &str,
    unexpected_result_message: &str,
) -> Result<(), ClassicError> {
    let object_id = resolve_classic_remote_object(
        state,
        binding,
        context,
        element_id,
        object_group,
        object_group,
    )
    .await?;
    let result = binding
        .runtime
        .execute_with_pending_navigation_wait_on_page(
            element_click_command(context, object_id.object_id.clone()),
            None,
            binding.timeouts.page_load.map(Duration::from_millis),
            object_id.page_residence.clone(),
        )
        .await;
    release_classic_page_bound_remote_object(binding, context, object_id, object_group).await;
    let post_click_navigation = classic_wait_for_current_document(binding).await;
    match result {
        Ok(DevToolsCommandResult::Script(result)) => {
            post_click_navigation?;
            match *result {
                DevToolsScriptResult::Value(value) => {
                    if value
                        .value
                        .get("detachedFrame")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        && binding.current_frame_id.is_some()
                    {
                        state.classic_session_registry.lock().set_current_frame_id(
                            &binding.session_id,
                            Some(format!(
                                "__moli-detached-frame__:{}",
                                binding.browsing_context_target_id()
                            )),
                        );
                    }
                    Ok(())
                }
                DevToolsScriptResult::Exception(exception) => {
                    Err(classic_webdriver_command_exception_error(exception))
                }
            }
        }
        Ok(_) => {
            post_click_navigation?;
            Err(ClassicError::new(
                ClassicErrorCode::UnknownError,
                unexpected_result_message,
            ))
        }
        // ChromeDriver treats execution-context loss caused by navigation as
        // a navigation hint and resolves it at the outer window-command
        // boundary. Moli emits this exact error only while consuming a
        // response from a command already dispatched to the old renderer;
        // replaying the JavaScript `.click()` against the successor Document
        // could activate a second element. Wait for that successor and accept
        // the side effect already dispatched in the old Document.
        //
        // Chromium references:
        // chrome/test/chromedriver/element_commands.cc (ExecuteClickElement)
        // chrome/test/chromedriver/window_commands.cc (ExecuteWindowCommand)
        Err(error) if error.message == "Renderer attachment changed" => post_click_navigation,
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

async fn webdriver_classic_element_is_file_input(
    state: &AppState,
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
) -> Result<bool, ClassicError> {
    let tag_name = webdriver_classic_element_property_value(
        state,
        binding,
        context,
        element_id,
        "localName",
        "send keys",
    )
    .await?;
    if tag_name.as_str() != Some("input") {
        return Ok(false);
    }
    let input_type = webdriver_classic_element_property_value(
        state,
        binding,
        context,
        element_id,
        "type",
        "send keys",
    )
    .await?;
    Ok(input_type
        .as_str()
        .is_some_and(|input_type| input_type.eq_ignore_ascii_case("file")))
}

async fn webdriver_classic_send_file_input_keys(
    state: &AppState,
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
    text: &str,
) -> Result<(), ClassicError> {
    let multiple = webdriver_classic_element_property_value(
        state,
        binding,
        context,
        element_id,
        "multiple",
        "send keys",
    )
    .await?
    .as_bool()
    .unwrap_or(false);
    let paths = classic_file_input_paths_from_send_keys(text)?;
    if !multiple && paths.len() > 1 {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "the element can not hold multiple files",
        ));
    }
    let files = selected_files_from_paths(&paths, "WebDriver Classic file upload")
        .map_err(|error| ClassicError::new(ClassicErrorCode::InvalidArgument, error.message))?;
    let object_id = resolve_classic_remote_object(
        state,
        binding,
        context,
        element_id,
        "webdriver-classic-file-upload",
        "element file upload",
    )
    .await?;
    let result = binding
        .runtime
        .execute_on_page(
            DevToolsCommand::SetFileInputFiles(DevToolsSetFileInputFilesCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverClassic,
                    session_id: Some(DevToolsSessionId::from(context.session_id.as_str())),
                    target_id: context.target_id.as_deref().map(DevToolsTargetId::from),
                    browser_context_id: None,
                },
                object_id: DevToolsRemoteHandleId::from(object_id.object_id.as_str()),
                files,
                append: multiple,
            }),
            object_id.page_residence.clone(),
        )
        .await;
    release_classic_page_bound_remote_object(binding, context, object_id, "element file upload")
        .await;
    match result {
        Ok(DevToolsCommandResult::Empty) => Ok(()),
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "element file upload returned an unexpected result",
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

fn classic_file_input_paths_from_send_keys(text: &str) -> Result<Vec<String>, ClassicError> {
    if text.is_empty() {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "text is empty",
        ));
    }
    let paths = text
        .split('\n')
        .map(|path| path.trim().to_owned())
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "text is empty",
        ));
    }
    Ok(paths)
}

async fn webdriver_classic_element_geometry(
    state: &AppState,
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    element_id: &str,
) -> Result<DevToolsDomGeometryResult, ClassicError> {
    let reference = resolve_classic_element_dom_reference(state, binding, element_id)?;
    let page_residence = reference.page_residence;
    let commands = element_click_prepare_reference_commands(context, reference.reference);
    let mut geometry = None;
    for command in commands {
        match binding
            .runtime
            .execute_on_page(command, page_residence.clone())
            .await
        {
            Ok(DevToolsCommandResult::Empty) => {}
            Ok(DevToolsCommandResult::DomGeometry(result)) => geometry = Some(result),
            Ok(_) => {
                return Err(ClassicError::new(
                    ClassicErrorCode::UnknownError,
                    "element geometry returned an unexpected result",
                ));
            }
            Err(error) => return Err(classic_error_from_devtools_error(error)),
        }
    }
    geometry.ok_or_else(|| {
        ClassicError::new(
            ClassicErrorCode::UnknownError,
            "element geometry did not return geometry",
        )
    })
}

async fn webdriver_classic_action_element_origin_points(
    state: &AppState,
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
    params: &serde_json::Value,
) -> Result<ClassicElementOriginViewportPoints, ClassicError> {
    let element_ids = action_element_origin_ids(params)?;
    let mut origins = ClassicElementOriginViewportPoints::new();
    for element_id in element_ids {
        let geometry =
            webdriver_classic_element_geometry(state, binding, context, &element_id).await?;
        let point = element_center_from_geometry(&geometry)?;
        origins.insert(element_id, point);
    }
    Ok(origins)
}

async fn webdriver_classic_viewport_bounds(
    binding: &ClassicSessionBinding,
    context: &ClassicDevToolsCommandContext,
) -> Result<ClassicViewportBounds, ClassicError> {
    match binding
        .runtime
        .execute(layout_metrics_command(context))
        .await
    {
        Ok(DevToolsCommandResult::LayoutMetrics(metrics)) => Ok(classic_viewport_bounds(metrics)),
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "layout metrics returned an unexpected result",
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

fn classic_viewport_bounds(metrics: DevToolsLayoutMetricsResult) -> ClassicViewportBounds {
    ClassicViewportBounds::new(
        metrics.layout_viewport_width,
        metrics.layout_viewport_height,
    )
}

async fn webdriver_classic_execute_empty_commands(
    binding: &ClassicSessionBinding,
    commands: Vec<DevToolsCommand>,
    unexpected_result_message: &str,
) -> Result<(), ClassicError> {
    for command in commands {
        webdriver_classic_expect_empty_result(
            binding.runtime.execute(command).await,
            unexpected_result_message,
        )?;
    }
    Ok(())
}

fn webdriver_classic_expect_empty_result(
    result: Result<DevToolsCommandResult, DevToolsError>,
    unexpected_result_message: &str,
) -> Result<(), ClassicError> {
    match result {
        Ok(DevToolsCommandResult::Empty) => Ok(()),
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            unexpected_result_message,
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

pub(super) async fn webdriver_classic_perform_actions(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_browsing_context(&binding);
    let input_context = classic_top_level_context(&binding);
    let element_origins =
        match webdriver_classic_action_element_origin_points(&state, &binding, &context, &params)
            .await
        {
            Ok(element_origins) => element_origins,
            Err(error) => return classic_error_into_response(error),
        };
    let viewport_bounds = match webdriver_classic_viewport_bounds(&binding, &input_context).await {
        Ok(viewport_bounds) => viewport_bounds,
        Err(error) => return classic_error_into_response(error),
    };
    let ticks = match state.classic_session_registry.lock().perform_actions_ticks(
        &session_id,
        &input_context,
        &params,
        &element_origins,
        viewport_bounds,
    ) {
        Ok(ticks) => ticks,
        Err(error) => return classic_error_into_response(error),
    };
    for tick in ticks {
        if let Err(error) = webdriver_classic_execute_empty_commands(
            &binding,
            tick.commands,
            "perform actions returned an unexpected result",
        )
        .await
        {
            return classic_error_into_response(error);
        }
        if tick.duration_ms > 0 {
            sleep(Duration::from_millis(tick.duration_ms)).await;
        }
    }
    classic_success_into_response(Value::Null)
}

pub(super) async fn webdriver_classic_release_actions(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = classic_top_level_context(&binding);
    let commands = match state
        .classic_session_registry
        .lock()
        .release_actions_commands(&session_id, &context)
    {
        Ok(commands) => commands,
        Err(error) => return classic_error_into_response(error),
    };
    if let Err(error) = webdriver_classic_execute_empty_commands(
        &binding,
        commands,
        "release actions returned an unexpected result",
    )
    .await
    {
        return classic_error_into_response(error);
    }
    classic_success_into_response(Value::Null)
}

fn webdriver_classic_capabilities(
    state: &AppState,
    session_id: &str,
    page_load_strategy: ClassicPageLoadStrategy,
    unhandled_prompt_behavior: &ClassicUnhandledPromptBehavior,
    downloads_enabled: bool,
) -> serde_json::Value {
    let web_socket_url = format!("{}/{}", state.bidi_ws_url.trim_end_matches('/'), session_id);
    let mut capabilities = json!({
        "browserName": "moli",
        "browserVersion": version::PRODUCT,
        "platformName": std::env::consts::OS,
        "acceptInsecureCerts": !state.fetch_config.tls_verify_host(),
        "pageLoadStrategy": page_load_strategy.as_str(),
        "unhandledPromptBehavior": unhandled_prompt_behavior.returned_capability(),
        "setWindowRect": false,
        "webSocketUrl": web_socket_url,
    });
    if downloads_enabled && let Some(object) = capabilities.as_object_mut() {
        object.insert("se:downloadsEnabled".to_owned(), json!(true));
    }
    capabilities
}

fn downloads_enabled_from_capabilities(
    capabilities: &serde_json::Map<String, Value>,
) -> Result<bool, ClassicError> {
    capabilities
        .get("se:downloadsEnabled")
        .map(downloads_enabled_capability_value)
        .unwrap_or(Ok(false))
}

fn downloads_enabled_capability_value(value: &Value) -> Result<bool, ClassicError> {
    value.as_bool().ok_or_else(|| {
        ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "se:downloadsEnabled must be a boolean",
        )
    })
}

fn selenium_download_behavior_command(session_id: &str, directory: &FsPath) -> DevToolsCommand {
    DevToolsCommand::SetDownloadBehavior(DevToolsSetDownloadBehaviorCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverClassic,
            session_id: Some(DevToolsSessionId::from(session_id)),
            target_id: None,
            browser_context_id: None,
        },
        behavior: Some(DevToolsDownloadBehaviorSetting {
            behavior: "allow".to_owned(),
            download_path: Some(directory.to_string_lossy().to_string()),
            events_enabled: true,
        }),
        user_contexts: None,
    })
}

fn classic_download_directory(state: &AppState, session_id: &str) -> Result<PathBuf, ClassicError> {
    let _binding = classic_session_binding(state, session_id)?;
    state
        .classic_session_registry
        .lock()
        .download_directory(session_id)
        .ok_or_else(|| {
            ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "Selenium downloads are not enabled for this session",
            )
        })
}

fn classic_download_file_names(directory: &FsPath) -> Result<Vec<String>, String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("unable to list download files: {error}")),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("unable to read download entry: {error}"))?;
        let metadata = entry
            .metadata()
            .map_err(|error| format!("unable to stat download entry: {error}"))?;
        if !metadata.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.ends_with(".crdownload") {
            continue;
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}

fn classic_download_file_name(name: &str) -> Result<String, ClassicError> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "invalid download file name",
        ));
    }
    Ok(name.to_owned())
}

fn classic_clear_download_directory(directory: &FsPath) -> Result<(), String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("unable to list download files: {error}")),
    };
    for entry in entries {
        let entry = entry.map_err(|error| format!("unable to read download entry: {error}"))?;
        let metadata = entry
            .metadata()
            .map_err(|error| format!("unable to stat download entry: {error}"))?;
        let path = entry.path();
        if metadata.is_file() {
            fs::remove_file(&path)
                .map_err(|error| format!("unable to remove download file: {error}"))?;
        } else if metadata.is_dir() {
            fs::remove_dir_all(&path)
                .map_err(|error| format!("unable to remove download directory: {error}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn described_shadow_root_prefers_backend_node_reference() {
        assert_eq!(
            classic_author_shadow_root_reference_from_described_node(&json!({
                "shadowRoots": [{
                    "nodeId": 7,
                    "backendNodeId": 42,
                    "shadowRootType": "open"
                }]
            })),
            Some((42, DevToolsDomNodeReference::BackendNodeId(42)))
        );
        assert_eq!(
            classic_author_shadow_root_reference_from_described_node(&json!({
                "shadowRoots": [{
                    "backendNodeId": 42,
                    "shadowRootType": "closed"
                }]
            })),
            Some((42, DevToolsDomNodeReference::BackendNodeId(42)))
        );
        assert_eq!(
            classic_author_shadow_root_reference_from_described_node(&json!({
                "shadowRoots": [{
                    "nodeId": 0,
                    "backendNodeId": 42,
                    "shadowRootType": "open"
                }]
            })),
            Some((42, DevToolsDomNodeReference::BackendNodeId(42)))
        );
    }

    #[test]
    fn described_user_agent_shadow_root_is_not_a_webdriver_shadow_root() {
        assert_eq!(
            classic_author_shadow_root_reference_from_described_node(&json!({
                "shadowRoots": [{
                    "nodeId": 7,
                    "backendNodeId": 42,
                    "shadowRootType": "user-agent"
                }]
            })),
            None
        );
    }

    #[test]
    fn frame_owner_reference_prefers_backend_node_id() {
        assert_eq!(
            classic_dom_reference_from_frame_owner_result(DevToolsGetFrameOwnerResult {
                node_id: 7,
                backend_node_id: 2_000_000_007,
            }),
            DevToolsDomNodeReference::BackendNodeId(2_000_000_007)
        );
        assert_eq!(
            classic_dom_reference_from_frame_owner_result(DevToolsGetFrameOwnerResult {
                node_id: 7,
                backend_node_id: 0,
            }),
            DevToolsDomNodeReference::FrontendNodeId(7)
        );
    }
}
