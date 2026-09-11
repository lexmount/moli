use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use moli_protocol::devtools_runtime::{
    DevToolsActivateTargetCommand, DevToolsAddNetworkDataCollectorCommand,
    DevToolsAddNetworkInterceptCommand, DevToolsAddPreloadScriptCommand,
    DevToolsAuthChallengeAction, DevToolsAuthCredentials, DevToolsBrowserContextId,
    DevToolsCallFunctionCommand, DevToolsCaptureScreenshotClip, DevToolsCaptureScreenshotCommand,
    DevToolsCloseTargetCommand, DevToolsCommand, DevToolsContinueInterceptedRequestCommand,
    DevToolsContinueInterceptedResponseCommand, DevToolsContinueWithAuthCommand,
    DevToolsCreateBrowserContextCommand, DevToolsDevicePixelRatioSetting,
    DevToolsDisownNetworkDataCommand, DevToolsDownloadBehaviorSetting,
    DevToolsEvaluateScriptCommand, DevToolsFailInterceptedRequestCommand, DevToolsFrameId,
    DevToolsFulfillInterceptedRequestCommand, DevToolsGeolocationOverride,
    DevToolsGeolocationOverrideState, DevToolsGetBrowserContextsCommand,
    DevToolsGetClientWindowsCommand, DevToolsGetFrameTreeCommand, DevToolsGetFrameTreesCommand,
    DevToolsGetNetworkDataCommand, DevToolsGetRealmsCommand, DevToolsHandleJavaScriptDialogCommand,
    DevToolsHistoryTraversalDestination, DevToolsLocateNodesCommand, DevToolsLocateNodesLocator,
    DevToolsLocateNodesTextMatch, DevToolsNavigationWait, DevToolsNetworkConditions,
    DevToolsNetworkDataCollectorId, DevToolsNetworkDataType, DevToolsNetworkInterceptId,
    DevToolsNetworkInterceptPattern, DevToolsNetworkInterceptPhase, DevToolsPreloadScriptId,
    DevToolsPreloadScriptSource, DevToolsPrintToPdfCommand, DevToolsPrintToPdfTransferMode,
    DevToolsRealmId, DevToolsReleaseObjectsCommand, DevToolsReloadCommand, DevToolsRemoteHandleId,
    DevToolsRemoveBrowserContextCommand, DevToolsRemoveNetworkDataCollectorCommand,
    DevToolsRemoveNetworkInterceptCommand, DevToolsRemovePreloadScriptCommand, DevToolsRequestId,
    DevToolsResultOwnership, DevToolsScreenshotClip, DevToolsScreenshotElementClip,
    DevToolsSerializationOptions, DevToolsSetCacheBehaviorCommand,
    DevToolsSetClientWindowStateCommand, DevToolsSetDownloadBehaviorCommand,
    DevToolsSetExtraHeadersCommand, DevToolsSetGeolocationOverrideCommand,
    DevToolsSetLocaleOverrideCommand, DevToolsSetNetworkConditionsCommand,
    DevToolsSetPermissionCommand, DevToolsSetTimezoneOverrideCommand,
    DevToolsSetUserAgentOverrideCommand, DevToolsSetViewportCommand, DevToolsTargetId,
    DevToolsTraverseHistoryCommand, DevToolsViewportSetting, DevToolsWindowState,
};
use serde_json::Value;

use crate::storage::{
    bidi_delete_cookies_command, bidi_get_cookies_command, bidi_set_cookie_command,
};
use crate::user_context::explicit_bidi_user_context_to_browser_context_id;
use crate::{BidiCommand, BidiDevToolsCommandContext, BidiError, BidiErrorCode};

const CENTIMETERS_PER_INCH: f64 = 2.54;
const MIN_PRINT_PAGE_SIZE_CM: f64 = CENTIMETERS_PER_INCH / 72.0;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

pub fn devtools_command_from_bidi_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsCommand, BidiError> {
    match command.method.as_str() {
        "browser.createUserContext" => Ok(DevToolsCommand::CreateBrowserContext(
            bidi_create_user_context_command(command, context)?,
        )),
        "browser.getUserContexts" => Ok(DevToolsCommand::GetBrowserContexts(
            DevToolsGetBrowserContextsCommand {
                context: context.command_context(None),
            },
        )),
        "browser.getClientWindows" => Ok(DevToolsCommand::GetClientWindows(
            DevToolsGetClientWindowsCommand {
                context: context.command_context(None),
            },
        )),
        "browser.setClientWindowState" => Ok(DevToolsCommand::SetClientWindowState(
            bidi_set_client_window_state_command(command, context)?,
        )),
        "browser.removeUserContext" => Ok(DevToolsCommand::RemoveBrowserContext(
            bidi_remove_user_context_command(command, context)?,
        )),
        "browser.setDownloadBehavior" => Ok(DevToolsCommand::SetDownloadBehavior(
            bidi_set_download_behavior_command(command, context)?,
        )),
        "permissions.setPermission" => Ok(DevToolsCommand::SetPermission(
            bidi_set_permission_command(command, context)?,
        )),
        "browsingContext.create" => {
            let type_name = required_string(&command.params, "type")?;
            if !matches!(type_name, "tab" | "window") {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "browsingContext.create type must be tab or window",
                ));
            }
            let reference_context =
                optional_string(&command.params, "referenceContext")?.map(DevToolsTargetId::from);
            let background = optional_bool(&command.params, "background")?.unwrap_or(false);
            let browser_context_id = bidi_create_user_context_browser_context_id(
                command,
                context,
                reference_context.as_ref(),
            )?;
            Ok(DevToolsCommand::CreateTarget(
                moli_protocol::devtools_runtime::DevToolsCreateTargetCommand {
                    context: context.command_context_with_browser_context_id(
                        reference_context.clone(),
                        browser_context_id.clone(),
                    ),
                    url: "about:blank".to_owned(),
                    browser_context_id,
                    activate: !background,
                },
            ))
        }
        "browsingContext.close" => {
            let target_id = required_string(&command.params, "context")?;
            validate_optional_bool(&command.params, "promptUnload")?;
            Ok(DevToolsCommand::CloseTarget(DevToolsCloseTargetCommand {
                context: context.command_context(Some(DevToolsTargetId::from(target_id))),
                target_id: DevToolsTargetId::from(target_id),
            }))
        }
        "browsingContext.activate" => {
            let target_id = required_string(&command.params, "context")?;
            Ok(DevToolsCommand::ActivateTarget(
                DevToolsActivateTargetCommand {
                    context: context.command_context(Some(DevToolsTargetId::from(target_id))),
                    target_id: DevToolsTargetId::from(target_id),
                },
            ))
        }
        "browsingContext.getTree" => bidi_get_tree_command(command, context),
        "browsingContext.locateNodes" => Ok(DevToolsCommand::LocateNodes(
            bidi_locate_nodes_command(command, context)?,
        )),
        "browsingContext.navigate" => {
            let target_id = required_string(&command.params, "context")?;
            let url = required_string(&command.params, "url")?;
            validate_bidi_navigation_url(url)?;
            Ok(DevToolsCommand::Navigate(
                moli_protocol::devtools_runtime::DevToolsNavigateCommand {
                    context: context.command_context(Some(DevToolsTargetId::from(target_id))),
                    url: url.to_owned(),
                    referrer: None,
                    wait: bidi_navigation_wait(command.params.get("wait"))?,
                },
            ))
        }
        "browsingContext.reload" => {
            let target_id = required_string(&command.params, "context")?;
            Ok(DevToolsCommand::Reload(DevToolsReloadCommand {
                context: context.command_context(Some(DevToolsTargetId::from(target_id))),
                ignore_cache: optional_bool(&command.params, "ignoreCache")?.unwrap_or(false),
                script_to_evaluate_on_load: None,
                wait: bidi_navigation_wait(command.params.get("wait"))?,
            }))
        }
        "browsingContext.traverseHistory" => Ok(DevToolsCommand::TraverseHistory(
            bidi_traverse_history_command(command, context)?,
        )),
        "browsingContext.handleUserPrompt" => Ok(DevToolsCommand::HandleJavaScriptDialog(
            bidi_handle_user_prompt_command(command, context)?,
        )),
        "browsingContext.captureScreenshot" => Ok(DevToolsCommand::CaptureScreenshot(
            bidi_capture_screenshot_command(command, context)?,
        )),
        "browsingContext.print" => Ok(DevToolsCommand::PrintToPdf(bidi_print_command(
            command, context,
        )?)),
        "browsingContext.setViewport" => Ok(DevToolsCommand::SetViewport(
            bidi_set_viewport_command(command, context)?,
        )),
        "emulation.setUserAgentOverride" => Ok(DevToolsCommand::SetUserAgentOverride(
            bidi_set_user_agent_override_command(command, context)?,
        )),
        "emulation.setLocaleOverride" => Ok(DevToolsCommand::SetLocaleOverride(
            bidi_set_locale_override_command(command, context)?,
        )),
        "emulation.setTimezoneOverride" => Ok(DevToolsCommand::SetTimezoneOverride(
            bidi_set_timezone_override_command(command, context)?,
        )),
        "emulation.setGeolocationOverride" => Ok(DevToolsCommand::SetGeolocationOverride(
            bidi_set_geolocation_override_command(command, context)?,
        )),
        "emulation.setNetworkConditions" => Ok(DevToolsCommand::SetNetworkConditions(
            bidi_set_network_conditions_command(command, context)?,
        )),
        "network.addIntercept" => Ok(DevToolsCommand::AddNetworkIntercept(
            bidi_network_add_intercept_command(command, context)?,
        )),
        "network.removeIntercept" => Ok(DevToolsCommand::RemoveNetworkIntercept(
            bidi_network_remove_intercept_command(command, context)?,
        )),
        "network.addDataCollector" => Ok(DevToolsCommand::AddNetworkDataCollector(
            bidi_network_add_data_collector_command(command, context)?,
        )),
        "network.removeDataCollector" => Ok(DevToolsCommand::RemoveNetworkDataCollector(
            bidi_network_remove_data_collector_command(command, context)?,
        )),
        "network.disownData" => Ok(DevToolsCommand::DisownNetworkData(
            bidi_network_disown_data_command(command, context)?,
        )),
        "network.getData" => Ok(DevToolsCommand::GetNetworkData(
            bidi_network_get_data_command(command, context)?,
        )),
        "network.setCacheBehavior" => Ok(DevToolsCommand::SetCacheBehavior(
            bidi_network_set_cache_behavior_command(command, context)?,
        )),
        "network.setExtraHeaders" => Ok(DevToolsCommand::SetExtraHeaders(
            bidi_network_set_extra_headers_command(command, context)?,
        )),
        "network.continueRequest" => Ok(DevToolsCommand::ContinueInterceptedRequest(
            bidi_network_continue_request_command(command, context)?,
        )),
        "network.continueResponse" => Ok(DevToolsCommand::ContinueInterceptedResponse(
            bidi_network_continue_response_command(command, context)?,
        )),
        "network.continueWithAuth" => Ok(DevToolsCommand::ContinueWithAuth(
            bidi_network_continue_with_auth_command(command, context)?,
        )),
        "network.failRequest" => Ok(DevToolsCommand::FailInterceptedRequest(
            bidi_network_fail_request_command(command, context)?,
        )),
        "network.provideResponse" => Ok(DevToolsCommand::FulfillInterceptedRequest(
            bidi_network_provide_response_command(command, context)?,
        )),
        "storage.getCookies" => Ok(DevToolsCommand::GetCookies(bidi_get_cookies_command(
            command, context,
        )?)),
        "storage.setCookie" => Ok(DevToolsCommand::SetCookies(bidi_set_cookie_command(
            command, context,
        )?)),
        "storage.deleteCookies" => Ok(DevToolsCommand::DeleteCookies(bidi_delete_cookies_command(
            command, context,
        )?)),
        "script.evaluate" => Ok(DevToolsCommand::EvaluateScript(
            bidi_evaluate_script_command(command, context)?,
        )),
        "script.callFunction" => Ok(DevToolsCommand::CallFunction(bidi_call_function_command(
            command, context,
        )?)),
        "script.getRealms" => Ok(DevToolsCommand::GetRealms(bidi_get_realms_command(
            command, context,
        )?)),
        "script.disown" => Ok(DevToolsCommand::ReleaseObjects(bidi_disown_command(
            command, context,
        )?)),
        "script.addPreloadScript" => Ok(DevToolsCommand::AddPreloadScript(
            bidi_add_preload_script_command(command, context)?,
        )),
        "script.removePreloadScript" => Ok(DevToolsCommand::RemovePreloadScript(
            bidi_remove_preload_script_command(command, context)?,
        )),
        method => Err(BidiError::new(BidiErrorCode::UnknownCommand, method)),
    }
}

fn bidi_create_user_context_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsCreateBrowserContextCommand, BidiError> {
    let proxy = bidi_create_user_context_proxy(&command.params)?;
    validate_unhandled_prompt_behavior(command.params.get("unhandledPromptBehavior"))?;
    Ok(DevToolsCreateBrowserContextCommand {
        context: context.command_context(None),
        browser_context_id: None,
        accept_insecure_certs: optional_bool(&command.params, "acceptInsecureCerts")?,
        proxy_server: proxy.proxy_server,
        proxy_bypass_list: proxy.proxy_bypass_list,
        proxy_autoconfig_url: proxy.proxy_autoconfig_url,
        proxy_socks_version: proxy.proxy_socks_version,
        persistent_partition_id: None,
    })
}

fn bidi_remove_user_context_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsRemoveBrowserContextCommand, BidiError> {
    let user_context = required_string(&command.params, "userContext")?;
    if user_context == crate::user_context::DEFAULT_BIDI_USER_CONTEXT {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "default user context cannot be removed",
        ));
    }
    Ok(DevToolsRemoveBrowserContextCommand {
        context: context.command_context(None),
        browser_context_id: DevToolsBrowserContextId::from(user_context),
    })
}

fn bidi_set_client_window_state_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsSetClientWindowStateCommand, BidiError> {
    let client_window = required_string(&command.params, "clientWindow")?;
    let state = match required_string(&command.params, "state")? {
        "normal" => DevToolsWindowState::Normal,
        "maximized" => DevToolsWindowState::Maximized,
        "minimized" => DevToolsWindowState::Minimized,
        "fullscreen" => DevToolsWindowState::Fullscreen,
        _ => {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "state must be normal, maximized, minimized, or fullscreen",
            ));
        }
    };
    let (width, height, x, y) = if state == DevToolsWindowState::Normal {
        (
            optional_uint(&command.params, "width")?,
            optional_uint(&command.params, "height")?,
            optional_int(&command.params, "x")?,
            optional_int(&command.params, "y")?,
        )
    } else {
        (None, None, None, None)
    };
    Ok(DevToolsSetClientWindowStateCommand {
        context: context.command_context(Some(DevToolsTargetId::from(client_window))),
        client_window: DevToolsTargetId::from(client_window),
        state,
        width,
        height,
        x,
        y,
    })
}

fn bidi_set_download_behavior_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsSetDownloadBehaviorCommand, BidiError> {
    let behavior = bidi_download_behavior_setting(command.params.get("downloadBehavior"))?;
    let user_contexts =
        optional_non_empty_string_array(&command.params, "userContexts")?.map(|user_contexts| {
            user_contexts
                .into_iter()
                .map(|user_context| explicit_bidi_user_context_to_browser_context_id(&user_context))
                .collect()
        });
    Ok(DevToolsSetDownloadBehaviorCommand {
        context: context.command_context(None),
        behavior,
        user_contexts,
    })
}

fn bidi_set_permission_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsSetPermissionCommand, BidiError> {
    let descriptor_value = command.params.get("descriptor").ok_or_else(|| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            "descriptor must be an object",
        )
    })?;
    let Some(descriptor) = descriptor_value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "descriptor must be an object",
        ));
    };
    let name = required_object_string(descriptor, "name")?;
    if !is_supported_bidi_permission_name(name) {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "descriptor.name is not supported",
        ));
    }
    let state = required_string(&command.params, "state")?;
    if !matches!(state, "granted" | "denied" | "prompt") {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "state must be granted, denied, or prompt",
        ));
    }
    let origin = required_string(&command.params, "origin")?;
    let browser_context_id = optional_string(&command.params, "userContext")?
        .map(explicit_bidi_user_context_to_browser_context_id);
    Ok(DevToolsSetPermissionCommand {
        context: context.command_context(None),
        permission: descriptor_value.clone(),
        setting: state.to_owned(),
        origin: origin.to_owned(),
        embedded_origin: optional_string(&command.params, "embeddedOrigin")?.map(str::to_owned),
        browser_context_id,
    })
}

fn is_supported_bidi_permission_name(name: &str) -> bool {
    matches!(
        name,
        "geolocation"
            | "storage-access"
            | "camera"
            | "microphone"
            | "notifications"
            | "clipboard-read"
            | "clipboard-write"
    )
}

fn bidi_download_behavior_setting(
    value: Option<&Value>,
) -> Result<Option<DevToolsDownloadBehaviorSetting>, BidiError> {
    let Some(value) = value else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "downloadBehavior must be an object or null",
        ));
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(behavior) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "downloadBehavior must be an object or null",
        ));
    };
    match required_object_string(behavior, "type")? {
        "allowed" => {
            let destination = required_object_string(behavior, "destinationFolder")?;
            if destination.is_empty() {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "destinationFolder must be a non-empty string",
                ));
            }
            Ok(Some(DevToolsDownloadBehaviorSetting {
                behavior: "allow".to_owned(),
                download_path: Some(destination.to_owned()),
                events_enabled: true,
            }))
        }
        "denied" => Ok(Some(DevToolsDownloadBehaviorSetting {
            behavior: "deny".to_owned(),
            download_path: None,
            events_enabled: true,
        })),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "downloadBehavior type must be allowed or denied",
        )),
    }
}

#[derive(Default)]
struct BidiProxySettings {
    proxy_server: Option<String>,
    proxy_bypass_list: Option<String>,
    proxy_autoconfig_url: Option<String>,
    proxy_socks_version: Option<u8>,
}

fn bidi_create_user_context_proxy(params: &Value) -> Result<BidiProxySettings, BidiError> {
    let Some(value) = params.get("proxy") else {
        return Ok(BidiProxySettings::default());
    };
    let Some(proxy) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "proxy must be an object",
        ));
    };
    let proxy_type = required_object_string(proxy, "proxyType")?;
    match proxy_type {
        "system" | "autodetect" | "direct" => Ok(BidiProxySettings::default()),
        "manual" => bidi_manual_proxy_settings(proxy),
        "pac" => {
            let proxy_autoconfig_url = required_object_string(proxy, "proxyAutoconfigUrl")?;
            Ok(BidiProxySettings {
                proxy_autoconfig_url: Some(proxy_autoconfig_url.to_owned()),
                ..BidiProxySettings::default()
            })
        }
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "proxyType is not supported",
        )),
    }
}

fn bidi_manual_proxy_settings(
    proxy: &serde_json::Map<String, Value>,
) -> Result<BidiProxySettings, BidiError> {
    let http_proxy = optional_proxy_address(proxy, "httpProxy")?;
    let ssl_proxy = optional_proxy_address(proxy, "sslProxy")?;
    let socks_proxy = optional_proxy_address(proxy, "socksProxy")?;
    let socks_version = optional_socks_version(proxy)?;
    if socks_proxy.is_some() != socks_version.is_some() {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "socksProxy and socksVersion must be specified together",
        ));
    }
    let no_proxy = optional_no_proxy(proxy)?;
    let socks_proxy_server = socks_proxy.as_ref().map(|proxy| {
        format!(
            "socks{}://{proxy}",
            socks_version.expect("socks proxy pairing should ensure a version")
        )
    });
    Ok(BidiProxySettings {
        proxy_server: http_proxy.or(ssl_proxy).or(socks_proxy_server),
        proxy_bypass_list: no_proxy,
        proxy_autoconfig_url: None,
        proxy_socks_version: socks_version,
    })
}

fn optional_proxy_address(
    proxy: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<String>, BidiError> {
    let Some(value) = optional_object_string(proxy, field)? else {
        return Ok(None);
    };
    validate_proxy_address(value, field)?;
    Ok(Some(value.to_owned()))
}

fn validate_proxy_address(value: &str, field: &str) -> Result<(), BidiError> {
    let invalid = || {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a host:port proxy address"),
        )
    };
    if value.contains("://") || value.contains('/') || value.contains('?') || value.contains('#') {
        return Err(invalid());
    }
    if let Some(rest) = value.strip_prefix('[') {
        let Some((host, port)) = rest.split_once("]:") else {
            return Err(invalid());
        };
        if host.is_empty() || port.is_empty() || port.parse::<u16>().is_err() {
            return Err(invalid());
        }
        return Ok(());
    }
    if value.matches(':').count() != 1 {
        return Err(invalid());
    }
    let Some((host, port)) = value.split_once(':') else {
        return Err(invalid());
    };
    if host.is_empty() || port.is_empty() || port.parse::<u16>().is_err() {
        return Err(invalid());
    }
    Ok(())
}

fn bidi_network_add_intercept_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsAddNetworkInterceptCommand, BidiError> {
    let phases = required_network_intercept_phases(&command.params)?;
    let url_patterns = optional_network_intercept_url_patterns(&command.params)?;
    let target_id = optional_network_intercept_context(&command.params)?;
    Ok(DevToolsAddNetworkInterceptCommand {
        context: context.command_context(target_id),
        intercept_id: DevToolsNetworkInterceptId::from(bidi_network_intercept_id(command.id)),
        phases,
        url_patterns,
    })
}

fn bidi_network_remove_intercept_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsRemoveNetworkInterceptCommand, BidiError> {
    Ok(DevToolsRemoveNetworkInterceptCommand {
        context: context.command_context(None),
        intercept_id: DevToolsNetworkInterceptId::from(required_string(
            &command.params,
            "intercept",
        )?),
    })
}

fn bidi_network_intercept_id(command_id: u64) -> String {
    format!(
        "00000000-0000-4000-8000-{suffix:012x}",
        suffix = command_id & 0x0000_ffff_ffff_ffff
    )
}

fn bidi_network_data_collector_id(command_id: u64) -> String {
    bidi_network_intercept_id(command_id)
}

fn bidi_network_add_data_collector_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsAddNetworkDataCollectorCommand, BidiError> {
    let scope = bidi_emulation_context_scope(command, false)?;
    validate_network_data_collector_type(&command.params)?;
    Ok(DevToolsAddNetworkDataCollectorCommand {
        context: context.command_context(None),
        collector_id: DevToolsNetworkDataCollectorId::from(bidi_network_data_collector_id(
            command.id,
        )),
        data_types: required_network_data_types(&command.params)?,
        max_encoded_data_size: required_positive_safe_u64(&command.params, "maxEncodedDataSize")?,
        target_ids: scope.target_ids,
        browser_context_ids: scope.browser_context_ids,
    })
}

fn bidi_network_remove_data_collector_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsRemoveNetworkDataCollectorCommand, BidiError> {
    Ok(DevToolsRemoveNetworkDataCollectorCommand {
        context: context.command_context(None),
        collector_id: DevToolsNetworkDataCollectorId::from(required_string(
            &command.params,
            "collector",
        )?),
    })
}

fn bidi_network_disown_data_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsDisownNetworkDataCommand, BidiError> {
    Ok(DevToolsDisownNetworkDataCommand {
        context: context.command_context(None),
        request_id: DevToolsRequestId::from(required_network_request_id(&command.params)?),
        data_type: required_network_data_type(&command.params)?,
        collector_id: DevToolsNetworkDataCollectorId::from(required_string(
            &command.params,
            "collector",
        )?),
    })
}

fn bidi_network_get_data_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsGetNetworkDataCommand, BidiError> {
    let data_type = required_network_data_type(&command.params)?;
    let collector =
        optional_string(&command.params, "collector")?.map(DevToolsNetworkDataCollectorId::from);
    let disown = optional_bool(&command.params, "disown")?.unwrap_or(false);
    if disown && collector.is_none() {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "network.getData disown requires collector",
        ));
    }
    Ok(DevToolsGetNetworkDataCommand {
        context: context.command_context(None),
        request_id: DevToolsRequestId::from(required_network_request_id(&command.params)?),
        data_type,
        collector,
        disown,
    })
}

fn required_network_data_type(params: &Value) -> Result<DevToolsNetworkDataType, BidiError> {
    match required_string(params, "dataType")? {
        "request" => Ok(DevToolsNetworkDataType::Request),
        "response" => Ok(DevToolsNetworkDataType::Response),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "dataType must be request or response",
        )),
    }
}

fn required_network_data_types(params: &Value) -> Result<Vec<DevToolsNetworkDataType>, BidiError> {
    let Some(value) = params.get("dataTypes") else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "dataTypes must be a non-empty array",
        ));
    };
    let Some(values) = value.as_array() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "dataTypes must be a non-empty array",
        ));
    };
    if values.is_empty() {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "dataTypes must be a non-empty array",
        ));
    }
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let data_type = match value.as_str() {
            Some("request") => DevToolsNetworkDataType::Request,
            Some("response") => DevToolsNetworkDataType::Response,
            Some(_) => {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "dataTypes entries must be request or response",
                ));
            }
            None => {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "dataTypes entries must be strings",
                ));
            }
        };
        if !out.contains(&data_type) {
            out.push(data_type);
        }
    }
    Ok(out)
}

fn validate_network_data_collector_type(params: &Value) -> Result<(), BidiError> {
    let Some(value) = params.get("collectorType") else {
        return Ok(());
    };
    match value.as_str() {
        Some("blob") => Ok(()),
        Some(_) | None => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "collectorType must be blob",
        )),
    }
}

fn required_positive_safe_u64(params: &Value, field: &str) -> Result<u64, BidiError> {
    let value = required_safe_integer(params, field)?;
    if value < 1 {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a positive integer"),
        ));
    }
    u64::try_from(value).map_err(|_| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a positive integer"),
        )
    })
}

fn required_network_intercept_phases(
    params: &Value,
) -> Result<Vec<DevToolsNetworkInterceptPhase>, BidiError> {
    let Some(value) = params.get("phases") else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "network.addIntercept phases are required",
        ));
    };
    let Some(values) = value.as_array() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "network.addIntercept phases must be an array",
        ));
    };
    if values.is_empty() {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "network.addIntercept phases must not be empty",
        ));
    }
    values
        .iter()
        .map(|value| {
            let Some(phase) = value.as_str() else {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "network.addIntercept phase must be a string",
                ));
            };
            match phase {
                "beforeRequestSent" => Ok(DevToolsNetworkInterceptPhase::BeforeRequestSent),
                "responseStarted" => Ok(DevToolsNetworkInterceptPhase::ResponseStarted),
                "authRequired" => Ok(DevToolsNetworkInterceptPhase::AuthRequired),
                _ => Err(BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "network.addIntercept phase is not supported",
                )),
            }
        })
        .collect()
}

fn optional_network_intercept_url_patterns(
    params: &Value,
) -> Result<Vec<DevToolsNetworkInterceptPattern>, BidiError> {
    let Some(value) = params.get("urlPatterns") else {
        return Ok(Vec::new());
    };
    let Some(patterns) = value.as_array() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "network.addIntercept urlPatterns must be an array",
        ));
    };
    patterns.iter().map(network_intercept_url_pattern).collect()
}

fn network_intercept_url_pattern(
    value: &Value,
) -> Result<DevToolsNetworkInterceptPattern, BidiError> {
    let Some(pattern) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "network.addIntercept urlPattern must be an object",
        ));
    };
    let type_name = required_object_string(pattern, "type")?;
    let url_pattern = match type_name {
        "string" => {
            let pattern = required_object_string(pattern, "pattern")?;
            validate_network_intercept_string_pattern(pattern)?.to_string()
        }
        "pattern" => network_intercept_pattern_object(pattern)?,
        _ => {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "network.addIntercept urlPattern type is not supported",
            ));
        }
    };
    Ok(DevToolsNetworkInterceptPattern { url_pattern })
}

fn validate_network_intercept_string_pattern(pattern: &str) -> Result<url::Url, BidiError> {
    let parsed = url::Url::parse(pattern).map_err(|_| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            "network.addIntercept string pattern must be a valid absolute URL",
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https" | "ws" | "wss") {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "network.addIntercept string pattern scheme is not supported",
        ));
    }
    Ok(parsed)
}

fn network_intercept_pattern_object(
    pattern: &serde_json::Map<String, Value>,
) -> Result<String, BidiError> {
    let protocol = optional_network_pattern_component(pattern, "protocol")?;
    let hostname = optional_network_pattern_component(pattern, "hostname")?;
    let port = optional_network_pattern_component(pattern, "port")?;
    let pathname = optional_network_pattern_component(pattern, "pathname")?;
    let search = optional_network_pattern_component(pattern, "search")?;
    if let Some(protocol) = protocol.as_deref() {
        validate_network_pattern_protocol(protocol)?;
    }
    if let Some(hostname) = hostname.as_deref() {
        validate_network_pattern_hostname(hostname)?;
    }
    if let Some(port) = port.as_deref() {
        validate_network_pattern_port(port)?;
    }
    if let Some(pathname) = pathname.as_deref()
        && (pathname.contains('?') || pathname.contains('#'))
    {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "network.addIntercept pathname is invalid",
        ));
    }
    if let Some(search) = search.as_deref()
        && search.contains('#')
    {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "network.addIntercept search is invalid",
        ));
    }
    let protocol = protocol
        .map(|protocol| protocol.to_ascii_lowercase())
        .unwrap_or_else(|| "*".to_owned());
    let hostname = hostname.unwrap_or_else(|| "*".to_owned());
    let port = port.map(|port| format!(":{port}")).unwrap_or_default();
    let pathname = pathname.unwrap_or_else(|| "*".to_owned());
    let search = search
        .map(|search| format!("?{search}"))
        .unwrap_or_default();
    Ok(format!("{protocol}://{hostname}{port}/{pathname}{search}"))
}

fn optional_network_pattern_component(
    pattern: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<String>, BidiError> {
    let Some(value) = pattern.get(field) else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("network.addIntercept {field} must be a string"),
        ));
    };
    Ok(Some(value.to_owned()))
}

fn validate_network_pattern_protocol(protocol: &str) -> Result<(), BidiError> {
    let lower = protocol.to_ascii_lowercase();
    if matches!(lower.as_str(), "http" | "https" | "ws" | "wss")
        && !protocol.contains(['/', '#', '@', '%'])
    {
        return Ok(());
    }
    Err(BidiError::new(
        BidiErrorCode::InvalidArgument,
        "network.addIntercept protocol is invalid",
    ))
}

fn validate_network_pattern_hostname(hostname: &str) -> Result<(), BidiError> {
    if !hostname.is_empty() && !hostname.contains(['/', '?', '#', ':']) && hostname != "::1" {
        return Ok(());
    }
    Err(BidiError::new(
        BidiErrorCode::InvalidArgument,
        "network.addIntercept hostname is invalid",
    ))
}

fn validate_network_pattern_port(port: &str) -> Result<(), BidiError> {
    if port.parse::<u16>().is_ok() {
        return Ok(());
    }
    Err(BidiError::new(
        BidiErrorCode::InvalidArgument,
        "network.addIntercept port is invalid",
    ))
}

fn optional_network_intercept_context(
    params: &Value,
) -> Result<Option<DevToolsTargetId>, BidiError> {
    let Some(contexts) = optional_non_empty_string_array(params, "contexts")? else {
        return Ok(None);
    };
    if contexts.len() > 1 {
        return Err(BidiError::new(
            BidiErrorCode::UnsupportedOperation,
            "network.addIntercept multiple contexts are not supported yet",
        ));
    }
    Ok(contexts.into_iter().next().map(DevToolsTargetId::from))
}

fn bidi_network_set_cache_behavior_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsSetCacheBehaviorCommand, BidiError> {
    let cache_behavior = required_string(&command.params, "cacheBehavior")?;
    let cache_disabled = match cache_behavior {
        "default" => false,
        "bypass" => true,
        _ => {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "network.setCacheBehavior cacheBehavior must be default or bypass",
            ));
        }
    };
    let target_ids = optional_non_empty_string_array(&command.params, "contexts")?
        .unwrap_or_default()
        .into_iter()
        .map(DevToolsTargetId::from)
        .collect();
    Ok(DevToolsSetCacheBehaviorCommand {
        context: context.command_context(None),
        target_ids,
        cache_disabled,
    })
}

fn bidi_network_set_extra_headers_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsSetExtraHeadersCommand, BidiError> {
    let scope = bidi_emulation_context_scope(command, false)?;
    Ok(DevToolsSetExtraHeadersCommand {
        context: context.command_context(None),
        target_ids: scope.target_ids,
        browser_context_ids: scope.browser_context_ids,
        headers: required_network_extra_headers(&command.params)?,
    })
}

fn required_network_extra_headers(params: &Value) -> Result<Vec<(String, String)>, BidiError> {
    let Some(value) = params.get("headers") else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "headers must be an array",
        ));
    };
    let Some(headers) = value.as_array() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "headers must be an array",
        ));
    };
    let mut out = Vec::with_capacity(headers.len());
    for header in headers {
        let (name, value) = network_extra_header_pair(header)?;
        out.retain(|(existing, _)| existing != &name);
        out.push((name, value));
    }
    Ok(out)
}

fn network_extra_header_pair(value: &Value) -> Result<(String, String), BidiError> {
    let Some(header) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "headers entries must be objects",
        ));
    };
    let name = required_object_string(header, "name")?;
    validate_network_header_name(name, "header name")?;
    let value = required_network_extra_header_value(required_object_value(header, "value")?)?;
    Ok((name.to_owned(), value))
}

fn required_network_extra_header_value(value: &Value) -> Result<String, BidiError> {
    let Some(value) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "value must be a network bytes value",
        ));
    };
    let type_name = required_object_string(value, "type")?;
    if type_name != "string" {
        return Err(BidiError::new(
            BidiErrorCode::UnsupportedOperation,
            "Only string headers values are supported",
        ));
    }
    let raw = required_object_string(value, "value")?;
    if raw.contains(['\0', '\n', '\r']) || raw.trim() != raw {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "header value is invalid",
        ));
    }
    Ok(raw.to_owned())
}

fn bidi_network_continue_request_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsContinueInterceptedRequestCommand, BidiError> {
    let request = required_network_request_id(&command.params)?;
    let mut headers = optional_network_headers(&command.params, "headers")?;
    apply_network_request_cookies(&mut headers, &command.params)?;
    Ok(DevToolsContinueInterceptedRequestCommand {
        context: context.command_context(None),
        request_id: DevToolsRequestId::from(request),
        url: optional_string(&command.params, "url")?.map(str::to_owned),
        method: optional_string(&command.params, "method")?.map(str::to_owned),
        post_data: optional_network_bytes_string(&command.params, "body")?,
        headers,
        intercept_response: optional_bool(&command.params, "interceptResponse")?.unwrap_or(false),
    })
}

fn bidi_network_continue_response_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsContinueInterceptedResponseCommand, BidiError> {
    let request = required_network_request_id(&command.params)?;
    let headers = optional_network_headers(&command.params, "headers")?;
    let cookie_headers = network_response_cookie_headers(&command.params)?;
    let response_headers = match (headers, cookie_headers.is_empty()) {
        (Some(mut headers), _) => {
            headers.extend(cookie_headers);
            Some(headers)
        }
        (None, false) => Some(cookie_headers),
        (None, true) => None,
    };
    Ok(DevToolsContinueInterceptedResponseCommand {
        context: context.command_context(None),
        request_id: DevToolsRequestId::from(request),
        response_code: optional_network_status_code(&command.params)?,
        response_headers,
        response_phrase: optional_string(&command.params, "reasonPhrase")?.map(str::to_owned),
        auth_credentials: optional_network_auth_credentials(&command.params)?,
    })
}

fn bidi_network_continue_with_auth_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsContinueWithAuthCommand, BidiError> {
    let request = required_network_request_id(&command.params)?;
    let action = required_string(&command.params, "action")?;
    let (action, username, password) = match action {
        "default" => (DevToolsAuthChallengeAction::Default, None, None),
        "cancel" => (DevToolsAuthChallengeAction::Cancel, None, None),
        "provideCredentials" => {
            let credentials = command.params.get("credentials").ok_or_else(|| {
                BidiError::new(BidiErrorCode::InvalidArgument, "credentials is required")
            })?;
            let credentials = network_auth_credentials(credentials)?;
            (
                DevToolsAuthChallengeAction::ProvideCredentials,
                Some(credentials.username),
                Some(credentials.password),
            )
        }
        _ => {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "action must be default, cancel, or provideCredentials",
            ));
        }
    };
    Ok(DevToolsContinueWithAuthCommand {
        context: context.command_context(None),
        request_id: DevToolsRequestId::from(request),
        action,
        username,
        password,
    })
}

fn optional_network_auth_credentials(
    params: &Value,
) -> Result<Option<DevToolsAuthCredentials>, BidiError> {
    params
        .get("credentials")
        .map(network_auth_credentials)
        .transpose()
}

fn network_auth_credentials(value: &Value) -> Result<DevToolsAuthCredentials, BidiError> {
    let Some(credentials) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "credentials must be an object",
        ));
    };
    let type_name = required_object_string(credentials, "type")?;
    if type_name != "password" {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "credentials type must be password",
        ));
    }
    Ok(DevToolsAuthCredentials {
        username: required_object_string(credentials, "username")?.to_owned(),
        password: required_object_string(credentials, "password")?.to_owned(),
    })
}

fn bidi_network_fail_request_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsFailInterceptedRequestCommand, BidiError> {
    Ok(DevToolsFailInterceptedRequestCommand {
        context: context.command_context(None),
        request_id: DevToolsRequestId::from(required_network_request_id(&command.params)?),
        error_text: "Failed".to_owned(),
    })
}

fn bidi_network_provide_response_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsFulfillInterceptedRequestCommand, BidiError> {
    let request = required_network_request_id(&command.params)?;
    let mut headers = optional_network_headers(&command.params, "headers")?.unwrap_or_default();
    headers.extend(network_response_cookie_headers(&command.params)?);
    Ok(DevToolsFulfillInterceptedRequestCommand {
        context: context.command_context(None),
        request_id: DevToolsRequestId::from(request),
        response_code: optional_network_status_code(&command.params)?.unwrap_or(200),
        response_headers: headers,
        body: optional_network_bytes(&command.params, "body")?,
        response_phrase: optional_string(&command.params, "reasonPhrase")?.map(str::to_owned),
    })
}

fn required_network_request_id(params: &Value) -> Result<String, BidiError> {
    let request = required_string(params, "request")?;
    Ok(request.to_owned())
}

fn optional_network_status_code(params: &Value) -> Result<Option<u16>, BidiError> {
    let Some(value) = params.get("statusCode") else {
        return Ok(None);
    };
    let Some(code) = value.as_u64() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "statusCode must be a uint",
        ));
    };
    if !(100..=999).contains(&code) {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "statusCode must be an HTTP status code",
        ));
    }
    Ok(Some(code as u16))
}

fn optional_network_bytes_string(params: &Value, field: &str) -> Result<Option<String>, BidiError> {
    params
        .get(field)
        .map(|value| required_network_bytes_value(value, field))
        .transpose()
}

fn optional_network_bytes(params: &Value, field: &str) -> Result<Option<Vec<u8>>, BidiError> {
    params
        .get(field)
        .map(|value| required_network_bytes(value, field))
        .transpose()
}

fn required_network_bytes(value: &Value, field: &str) -> Result<Vec<u8>, BidiError> {
    let Some(value) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a network bytes value"),
        ));
    };
    let type_name = required_object_string(value, "type")?;
    let raw = required_object_string(value, "value")?;
    match type_name {
        "string" => Ok(raw.as_bytes().to_vec()),
        "base64" => BASE64_STANDARD.decode(raw).map_err(|_| {
            BidiError::new(
                BidiErrorCode::InvalidArgument,
                format!("{field} base64 value is invalid"),
            )
        }),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} type must be string or base64"),
        )),
    }
}

fn optional_network_headers(
    params: &Value,
    field: &str,
) -> Result<Option<Vec<(String, String)>>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    let Some(headers) = value.as_array() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be an array"),
        ));
    };
    let mut out = Vec::with_capacity(headers.len());
    for header in headers {
        out.push(network_header_pair(header, field)?);
    }
    Ok(Some(out))
}

fn network_header_pair(value: &Value, field: &str) -> Result<(String, String), BidiError> {
    let Some(header) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} entries must be objects"),
        ));
    };
    let name = required_object_string(header, "name")?;
    validate_network_header_name(name, "header name")?;
    let value = required_network_bytes_value(required_object_value(header, "value")?, "value")?;
    Ok((name.to_owned(), value))
}

fn apply_network_request_cookies(
    headers: &mut Option<Vec<(String, String)>>,
    params: &Value,
) -> Result<(), BidiError> {
    let Some(cookie_header) = optional_network_request_cookie_header(params)? else {
        return Ok(());
    };
    let headers = headers.get_or_insert_with(Vec::new);
    headers.retain(|(name, _)| !name.eq_ignore_ascii_case("cookie"));
    headers.push(("Cookie".to_owned(), cookie_header));
    Ok(())
}

fn optional_network_request_cookie_header(params: &Value) -> Result<Option<String>, BidiError> {
    let Some(cookies) = params.get("cookies") else {
        return Ok(None);
    };
    let Some(cookies) = cookies.as_array() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "cookies must be an array",
        ));
    };
    let mut values = Vec::with_capacity(cookies.len());
    for cookie in cookies {
        let (name, value) = network_cookie_name_value(cookie)?;
        values.push(format!("{name}={value}"));
    }
    Ok(Some(values.join("; ")))
}

fn network_response_cookie_headers(params: &Value) -> Result<Vec<(String, String)>, BidiError> {
    let Some(cookies) = params.get("cookies") else {
        return Ok(Vec::new());
    };
    let Some(cookies) = cookies.as_array() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "cookies must be an array",
        ));
    };
    let mut headers = Vec::with_capacity(cookies.len());
    for cookie in cookies {
        headers.push(("Set-Cookie".to_owned(), network_set_cookie_header(cookie)?));
    }
    Ok(headers)
}

fn network_cookie_name_value(value: &Value) -> Result<(String, String), BidiError> {
    let Some(cookie) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "cookies entries must be objects",
        ));
    };
    let name = required_object_string(cookie, "name")?;
    validate_network_cookie_name(name)?;
    let value = required_network_bytes_value(required_object_value(cookie, "value")?, "value")?;
    Ok((name.to_owned(), value))
}

fn network_set_cookie_header(value: &Value) -> Result<String, BidiError> {
    let Some(cookie) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "cookies entries must be objects",
        ));
    };
    let (name, value) = network_cookie_name_value(value)?;
    let mut header = format!("{name}={value}");
    for field in ["domain", "expiry", "path"] {
        if let Some(value) = optional_object_string(cookie, field)? {
            let attribute = match field {
                "domain" => "Domain",
                "expiry" => "Expires",
                "path" => "Path",
                _ => unreachable!(),
            };
            header.push_str(&format!("; {attribute}={value}"));
        }
    }
    if let Some(max_age) = optional_object_uint(cookie, "maxAge")? {
        header.push_str(&format!("; Max-Age={max_age}"));
    }
    if let Some(same_site) = optional_object_string(cookie, "sameSite")? {
        if !matches!(
            same_site,
            "strict" | "lax" | "none" | "Strict" | "Lax" | "None"
        ) {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "sameSite must be strict, lax, or none",
            ));
        }
        header.push_str(&format!("; SameSite={same_site}"));
    }
    if optional_object_bool(cookie, "httpOnly")?.unwrap_or(false) {
        header.push_str("; HttpOnly");
    }
    if optional_object_bool(cookie, "secure")?.unwrap_or(false) {
        header.push_str("; Secure");
    }
    Ok(header)
}

fn validate_network_header_name(name: &str, label: &str) -> Result<(), BidiError> {
    if name.is_empty()
        || !name.is_ascii()
        || name.bytes().any(|byte| {
            byte <= 0x20
                || byte >= 0x7f
                || matches!(
                    byte,
                    b'"' | b'#'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'('
                        | b')'
                        | b'*'
                        | b','
                        | b'/'
                        | b':'
                        | b';'
                        | b'<'
                        | b'='
                        | b'>'
                        | b'?'
                        | b'@'
                        | b'['
                        | b'\\'
                        | b']'
                        | b'`'
                        | b'{'
                        | b'|'
                        | b'}'
                )
        })
    {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{label} is invalid"),
        ));
    }
    Ok(())
}

fn validate_network_cookie_name(name: &str) -> Result<(), BidiError> {
    validate_network_header_name(name, "cookie name")
}

fn optional_socks_version(proxy: &serde_json::Map<String, Value>) -> Result<Option<u8>, BidiError> {
    let Some(value) = proxy.get("socksVersion") else {
        return Ok(None);
    };
    let Some(version) = value.as_u64() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "socksVersion must be a uint",
        ));
    };
    let version = u8::try_from(version).map_err(|_| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            "socksVersion must be 4 or 5",
        )
    })?;
    if !matches!(version, 4 | 5) {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "socksVersion must be 4 or 5",
        ));
    }
    Ok(Some(version))
}

fn optional_no_proxy(proxy: &serde_json::Map<String, Value>) -> Result<Option<String>, BidiError> {
    let Some(value) = proxy.get("noProxy") else {
        return Ok(None);
    };
    let Some(entries) = value.as_array() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "noProxy must be an array",
        ));
    };
    let mut no_proxy = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(entry) = entry.as_str() else {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "noProxy entries must be strings",
            ));
        };
        no_proxy.push(entry.to_owned());
    }
    Ok((!no_proxy.is_empty()).then(|| no_proxy.join(",")))
}

fn validate_unhandled_prompt_behavior(value: Option<&Value>) -> Result<(), BidiError> {
    let Some(value) = value else {
        return Ok(());
    };
    let Some(behavior) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "unhandledPromptBehavior must be an object",
        ));
    };
    for handler in [
        "alert",
        "beforeUnload",
        "confirm",
        "default",
        "file",
        "prompt",
    ] {
        let Some(value) = behavior.get(handler) else {
            continue;
        };
        let Some(value) = value.as_str() else {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "unhandledPromptBehavior handler must be a string",
            ));
        };
        if !matches!(value, "accept" | "dismiss" | "ignore") {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "unhandledPromptBehavior handler value is not supported",
            ));
        }
    }
    Ok(())
}

fn bidi_create_user_context_browser_context_id(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
    reference_context: Option<&DevToolsTargetId>,
) -> Result<Option<DevToolsBrowserContextId>, BidiError> {
    Ok(match optional_string(&command.params, "userContext")? {
        Some(user_context) => Some(explicit_bidi_user_context_to_browser_context_id(
            user_context,
        )),
        None if reference_context.is_some() => None,
        None => context
            .browser_context_id
            .as_deref()
            .map(DevToolsBrowserContextId::from),
    })
}

fn bidi_get_tree_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsCommand, BidiError> {
    let root = optional_nullable_string(&command.params, "root")?.map(DevToolsTargetId::from);
    let max_depth = bidi_get_tree_max_depth(&command.params)?;
    if let Some(root) = root {
        return Ok(DevToolsCommand::GetFrameTree(DevToolsGetFrameTreeCommand {
            context: context.command_context(Some(root)),
            max_depth,
        }));
    }
    Ok(DevToolsCommand::GetFrameTrees(
        DevToolsGetFrameTreesCommand {
            context: context.command_context(None),
            max_depth,
        },
    ))
}

fn bidi_get_tree_max_depth(params: &Value) -> Result<Option<u32>, BidiError> {
    match params.get("maxDepth") {
        Some(value) => {
            let Some(depth) = value.as_u64() else {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "browsingContext.getTree maxDepth must be a uint",
                ));
            };
            Ok(Some(u32::try_from(depth).map_err(|_| {
                BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "browsingContext.getTree maxDepth is too large",
                )
            })?))
        }
        None => Ok(None),
    }
}

fn bidi_locate_nodes_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsLocateNodesCommand, BidiError> {
    let context_id = required_string(&command.params, "context")?;
    let locator = bidi_locate_nodes_locator(&command.params)?;
    let start_nodes = bidi_locate_nodes_start_nodes(command.params.get("startNodes"))?;
    if matches!(locator, DevToolsLocateNodesLocator::Context(_)) && !start_nodes.is_empty() {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "Start nodes are not supported",
        ));
    }
    Ok(DevToolsLocateNodesCommand {
        context: context.command_context(Some(DevToolsTargetId::from(context_id))),
        locator,
        max_node_count: bidi_locate_nodes_max_node_count(command.params.get("maxNodeCount"))?,
        start_nodes,
        start_node_references: Vec::new(),
        serialization_options: bidi_serialization_options(&command.params)?,
    })
}

fn bidi_locate_nodes_locator(params: &Value) -> Result<DevToolsLocateNodesLocator, BidiError> {
    let locator = params
        .get("locator")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            BidiError::new(
                BidiErrorCode::InvalidArgument,
                "browsingContext.locateNodes locator must be an object",
            )
        })?;
    let locator_type = locator.get("type").and_then(Value::as_str).ok_or_else(|| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.locateNodes locator type must be a string",
        )
    })?;
    let value = locator.get("value").ok_or_else(|| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.locateNodes locator value is required",
        )
    })?;

    match locator_type {
        "css" => Ok(DevToolsLocateNodesLocator::Css(
            value
                .as_str()
                .ok_or_else(|| {
                    BidiError::new(
                        BidiErrorCode::InvalidArgument,
                        "browsingContext.locateNodes css locator value must be a string",
                    )
                })?
                .to_owned(),
        )),
        "xpath" => {
            let value = value.as_str().ok_or_else(|| {
                BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "browsingContext.locateNodes xpath locator value must be a string",
                )
            })?;
            if value.is_empty() {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidSelector,
                    "xpath locator cannot be empty",
                ));
            }
            Ok(DevToolsLocateNodesLocator::XPath(value.to_owned()))
        }
        "context" => {
            let value = value.as_object().ok_or_else(|| {
                BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "browsingContext.locateNodes context locator value must be an object",
                )
            })?;
            let context = value
                .get("context")
                .and_then(Value::as_str)
                .ok_or_else(|| BidiError::new(BidiErrorCode::InvalidSelector, "Invalid context"))?;
            if context.is_empty() {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidSelector,
                    "Invalid context",
                ));
            }
            Ok(DevToolsLocateNodesLocator::Context(DevToolsFrameId::from(
                context.to_owned(),
            )))
        }
        "innerText" => {
            let value = value.as_str().ok_or_else(|| {
                BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "browsingContext.locateNodes innerText locator value must be a string",
                )
            })?;
            if value.is_empty() {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidSelector,
                    "innerText locator cannot be empty",
                ));
            }
            Ok(DevToolsLocateNodesLocator::InnerText {
                value: value.to_owned(),
                ignore_case: optional_object_bool(locator, "ignoreCase")?.unwrap_or(false),
                match_type: bidi_locate_nodes_text_match(locator.get("matchType"))?,
                max_depth: optional_object_uint(locator, "maxDepth")?.unwrap_or(1000),
            })
        }
        "accessibility" => {
            let value = value.as_object().ok_or_else(|| {
                BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "browsingContext.locateNodes accessibility locator value must be an object",
                )
            })?;
            let role = optional_object_string(value, "role")?.map(str::to_owned);
            let name = optional_object_string(value, "name")?.map(str::to_owned);
            if role.is_none() && name.is_none() {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidSelector,
                    "accessibility locator must specify role or name",
                ));
            }
            Ok(DevToolsLocateNodesLocator::Accessibility { role, name })
        }
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.locateNodes locator type is not supported",
        )),
    }
}

fn bidi_locate_nodes_text_match(
    value: Option<&Value>,
) -> Result<DevToolsLocateNodesTextMatch, BidiError> {
    let Some(value) = value else {
        return Ok(DevToolsLocateNodesTextMatch::Full);
    };
    match value.as_str() {
        Some("full") => Ok(DevToolsLocateNodesTextMatch::Full),
        Some("partial") => Ok(DevToolsLocateNodesTextMatch::Partial),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.locateNodes innerText matchType must be full or partial",
        )),
    }
}

fn bidi_locate_nodes_max_node_count(value: Option<&Value>) -> Result<Option<u64>, BidiError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(count) = value.as_u64() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.locateNodes maxNodeCount must be a uint",
        ));
    };
    if count == 0 || count > MAX_SAFE_INTEGER as u64 {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.locateNodes maxNodeCount must be between 1 and 9007199254740991",
        ));
    }
    Ok(Some(count))
}

fn bidi_locate_nodes_start_nodes(value: Option<&Value>) -> Result<Vec<Value>, BidiError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(nodes) = value.as_array() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.locateNodes startNodes must be an array",
        ));
    };
    if nodes.is_empty() {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.locateNodes startNodes must not be empty",
        ));
    }
    for node in nodes {
        let Some(object) = node.as_object() else {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "browsingContext.locateNodes startNodes entries must be node remote values",
            ));
        };
        if object.get("type").and_then(Value::as_str) != Some("node")
            || object
                .get("sharedId")
                .or_else(|| object.get("handle"))
                .and_then(Value::as_str)
                .is_none()
        {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "browsingContext.locateNodes startNodes entries must be node remote values",
            ));
        }
    }
    Ok(nodes.clone())
}

fn optional_nullable_string<'a>(
    params: &'a Value,
    field: &str,
) -> Result<Option<&'a str>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value.as_str().map(Some).ok_or_else(|| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a string or null"),
        )
    })
}

fn bidi_evaluate_script_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsEvaluateScriptCommand, BidiError> {
    let expression = required_string(&command.params, "expression")?.to_owned();
    let (target_id, realm_id, world_name) = bidi_script_target(command.params.get("target"))?;
    validate_optional_serialization_options(&command.params)?;
    let user_activation = optional_bool(&command.params, "userActivation")?;
    let await_promise = optional_bool(&command.params, "awaitPromise")?.unwrap_or(false);
    let result_ownership = bidi_result_ownership(command.params.get("resultOwnership"))?;
    let serialization_options = Some(bidi_script_serialization_options(&command.params)?);
    let preserve_remote_metadata = should_preserve_remote_metadata(
        result_ownership,
        await_promise,
        serialization_options.as_ref(),
    );
    Ok(DevToolsEvaluateScriptCommand {
        context: context.command_context(target_id),
        realm_id,
        world_name,
        expression,
        await_promise,
        user_gesture: user_activation.unwrap_or(false),
        webdriver_bidi_file_prompt_handler: None,
        result_ownership,
        preserve_remote_metadata,
        serialization_options,
        materialize_bidi_script_result: true,
    })
}

fn bidi_call_function_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsCallFunctionCommand, BidiError> {
    let function_declaration = required_string(&command.params, "functionDeclaration")?.to_owned();
    let (target_id, realm_id, world_name) = bidi_script_target(command.params.get("target"))?;
    validate_optional_script_argument(&command.params, "this")?;
    validate_optional_serialization_options(&command.params)?;
    let user_activation = optional_bool(&command.params, "userActivation")?;
    let await_promise = optional_bool(&command.params, "awaitPromise")?.unwrap_or(false);
    let result_ownership = bidi_result_ownership(command.params.get("resultOwnership"))?;
    let serialization_options = Some(bidi_script_serialization_options(&command.params)?);
    let preserve_remote_metadata = should_preserve_remote_metadata(
        result_ownership,
        await_promise,
        serialization_options.as_ref(),
    );
    Ok(DevToolsCallFunctionCommand {
        context: context.command_context(target_id),
        realm_id,
        world_name,
        object_id: None,
        this_parameter: command.params.get("this").cloned(),
        function_declaration,
        arguments: optional_script_argument_array(&command.params, "arguments")?,
        await_promise,
        user_gesture: user_activation.unwrap_or(false),
        webdriver_bidi_file_prompt_handler: None,
        result_ownership,
        object_group: None,
        preserve_remote_metadata,
        serialization_options,
        materialize_bidi_script_result: true,
    })
}

fn should_preserve_remote_metadata(
    result_ownership: DevToolsResultOwnership,
    _await_promise: bool,
    _serialization_options: Option<&DevToolsSerializationOptions>,
) -> bool {
    matches!(result_ownership, DevToolsResultOwnership::None)
}

fn bidi_disown_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsReleaseObjectsCommand, BidiError> {
    let (target_id, realm_id, world_name) = bidi_script_target(command.params.get("target"))?;
    let handles = optional_string_array(&command.params, "handles")?
        .ok_or_else(|| BidiError::new(BidiErrorCode::InvalidArgument, "handles must be an array"))?
        .into_iter()
        .map(DevToolsRemoteHandleId::from)
        .collect();
    Ok(DevToolsReleaseObjectsCommand {
        context: context.command_context(target_id),
        realm_id,
        world_name,
        handles,
    })
}

fn bidi_get_realms_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsGetRealmsCommand, BidiError> {
    let target_id = optional_string(&command.params, "context")?.map(DevToolsTargetId::from);
    let realm_type = optional_string(&command.params, "type")?.map(str::to_owned);
    if let Some(realm_type) = realm_type.as_deref()
        && !is_bidi_realm_type(realm_type)
    {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "script.getRealms type must be a valid realm type",
        ));
    }
    Ok(DevToolsGetRealmsCommand {
        context: context.command_context(target_id),
        realm_type,
    })
}

fn bidi_add_preload_script_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsAddPreloadScriptCommand, BidiError> {
    if command.params.get("contexts").is_some() && command.params.get("userContexts").is_some() {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "script.addPreloadScript cannot specify both contexts and userContexts",
        ));
    }
    let target_ids =
        optional_non_empty_string_array(&command.params, "contexts")?.map(|contexts| {
            contexts
                .into_iter()
                .map(DevToolsTargetId::from)
                .collect::<Vec<_>>()
        });
    let browser_context_ids = optional_non_empty_string_array(&command.params, "userContexts")?
        .unwrap_or_default()
        .into_iter()
        .map(DevToolsBrowserContextId::from)
        .collect::<Vec<_>>();
    let context_target = target_ids
        .as_ref()
        .and_then(|target_ids| target_ids.first().cloned());
    Ok(DevToolsAddPreloadScriptCommand {
        context: context.command_context(context_target),
        source: DevToolsPreloadScriptSource::FunctionDeclaration {
            function_declaration: required_string(&command.params, "functionDeclaration")?
                .to_owned(),
            arguments: optional_channel_argument_array(&command.params, "arguments")?,
        },
        world_name: optional_string(&command.params, "sandbox")?.map(str::to_owned),
        target_ids,
        browser_context_ids,
        run_immediately: false,
        include_command_line_api: false,
    })
}

fn bidi_remove_preload_script_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsRemovePreloadScriptCommand, BidiError> {
    Ok(DevToolsRemovePreloadScriptCommand {
        context: context.command_context(None),
        script_id: DevToolsPreloadScriptId::from(required_string(&command.params, "script")?),
    })
}

fn bidi_script_target(
    target: Option<&Value>,
) -> Result<
    (
        Option<DevToolsTargetId>,
        Option<DevToolsRealmId>,
        Option<String>,
    ),
    BidiError,
> {
    let Some(target) = target.and_then(Value::as_object) else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "script target must be an object",
        ));
    };
    if let Some(sandbox) = target.get("sandbox")
        && !sandbox.is_string()
    {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "script target sandbox must be a string",
        ));
    }
    let context_id = target.get("context").and_then(Value::as_str);
    let realm_id = target.get("realm").and_then(Value::as_str);
    let world_name = target
        .get("sandbox")
        .and_then(Value::as_str)
        .filter(|sandbox| !sandbox.is_empty())
        .map(str::to_owned);
    match (context_id, realm_id) {
        (Some(context_id), _) => Ok((Some(DevToolsTargetId::from(context_id)), None, world_name)),
        (None, Some(realm_id)) if world_name.is_none() => {
            Ok((None, Some(DevToolsRealmId::from(realm_id)), None))
        }
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "script target must contain exactly one of context or realm; sandbox requires context",
        )),
    }
}

fn bidi_navigation_wait(wait: Option<&Value>) -> Result<DevToolsNavigationWait, BidiError> {
    let Some(wait) = wait else {
        return Ok(DevToolsNavigationWait::None);
    };
    let Some(wait) = wait.as_str() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.navigate wait must be none, interactive, or complete",
        ));
    };
    match wait {
        "none" => Ok(DevToolsNavigationWait::None),
        "interactive" => Ok(DevToolsNavigationWait::DomContentLoaded),
        "complete" => Ok(DevToolsNavigationWait::Load),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.navigate wait must be none, interactive, or complete",
        )),
    }
}

fn validate_bidi_navigation_url(url: &str) -> Result<(), BidiError> {
    url::Url::parse(url).map(|_| ()).map_err(|_| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.navigate url must be an absolute URL",
        )
    })
}

fn bidi_traverse_history_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsTraverseHistoryCommand, BidiError> {
    let target_id = required_string(&command.params, "context")?;
    let delta = required_safe_integer(&command.params, "delta")?;
    Ok(DevToolsTraverseHistoryCommand {
        context: context.command_context(Some(DevToolsTargetId::from(target_id))),
        destination: DevToolsHistoryTraversalDestination::Delta(delta),
        wait: DevToolsNavigationWait::Load,
    })
}

fn bidi_handle_user_prompt_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsHandleJavaScriptDialogCommand, BidiError> {
    let target_id = required_string(&command.params, "context")?;
    Ok(DevToolsHandleJavaScriptDialogCommand {
        context: context.command_context(Some(DevToolsTargetId::from(target_id))),
        accept: optional_bool(&command.params, "accept")?.unwrap_or(true),
        prompt_text: optional_string(&command.params, "userText")?
            .unwrap_or_default()
            .to_owned(),
    })
}

fn bidi_capture_screenshot_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsCaptureScreenshotCommand, BidiError> {
    let target_id = required_string(&command.params, "context")?;
    let format = bidi_capture_screenshot_format(command.params.get("format"))?;
    validate_bidi_capture_screenshot_origin(command.params.get("origin"))?;
    let clip = bidi_capture_screenshot_clip(command.params.get("clip"))?;
    Ok(DevToolsCaptureScreenshotCommand {
        context: context.command_context(Some(DevToolsTargetId::from(target_id))),
        format,
        quality: None,
        clip,
        capture_beyond_viewport: true,
        optimize_for_speed: false,
    })
}

fn bidi_capture_screenshot_format(format: Option<&Value>) -> Result<Option<String>, BidiError> {
    let Some(format) = format else {
        return Ok(Some("png".to_owned()));
    };
    let Some(format) = format.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.captureScreenshot format must be an object",
        ));
    };
    let type_name = required_object_string(format, "type")?;
    if let Some(quality) = format.get("quality") {
        let Some(quality) = quality.as_f64() else {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "browsingContext.captureScreenshot format quality must be a number",
            ));
        };
        if !(0.0..=1.0).contains(&quality) {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "browsingContext.captureScreenshot format quality must be between 0 and 1",
            ));
        }
    }
    match type_name {
        "image/png" => Ok(Some("png".to_owned())),
        "image/jpeg" => Ok(Some("jpeg".to_owned())),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.captureScreenshot format type must be image/png or image/jpeg",
        )),
    }
}

fn validate_bidi_capture_screenshot_origin(origin: Option<&Value>) -> Result<(), BidiError> {
    let Some(origin) = origin else {
        return Ok(());
    };
    let Some(origin) = origin.as_str() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.captureScreenshot origin must be viewport or document",
        ));
    };
    match origin {
        "viewport" | "document" => Ok(()),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.captureScreenshot origin must be viewport or document",
        )),
    }
}

fn bidi_capture_screenshot_clip(
    clip: Option<&Value>,
) -> Result<Option<DevToolsCaptureScreenshotClip>, BidiError> {
    let Some(clip) = clip else {
        return Ok(None);
    };
    let Some(clip) = clip.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.captureScreenshot clip must be an object",
        ));
    };
    let type_name = required_object_string(clip, "type")?;
    match type_name {
        "box" => Ok(Some(DevToolsCaptureScreenshotClip::Box(
            DevToolsScreenshotClip {
                x: required_object_finite_number(clip, "x")?,
                y: required_object_finite_number(clip, "y")?,
                width: required_object_finite_number(clip, "width")?,
                height: required_object_finite_number(clip, "height")?,
                scale: 1.0,
            },
        ))),
        "element" => bidi_capture_screenshot_element_clip(clip),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.captureScreenshot clip type must be box or element",
        )),
    }
}

fn bidi_capture_screenshot_element_clip(
    clip: &serde_json::Map<String, Value>,
) -> Result<Option<DevToolsCaptureScreenshotClip>, BidiError> {
    let Some(element) = clip.get("element").and_then(Value::as_object) else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.captureScreenshot element clip element must be an object",
        ));
    };
    let Some(shared_id) = element.get("sharedId").and_then(Value::as_str) else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.captureScreenshot element clip sharedId must be a string",
        ));
    };
    Ok(Some(DevToolsCaptureScreenshotClip::Element(
        DevToolsScreenshotElementClip {
            shared_id: DevToolsRemoteHandleId::from(shared_id),
        },
    )))
}

fn bidi_print_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsPrintToPdfCommand, BidiError> {
    let target_id = required_string(&command.params, "context")?;
    let margin = bidi_print_margin(command.params.get("margin"))?;
    let page = bidi_print_page(command.params.get("page"))?;
    Ok(DevToolsPrintToPdfCommand {
        context: context.command_context(Some(DevToolsTargetId::from(target_id))),
        landscape: bidi_print_orientation(command.params.get("orientation"))?,
        print_background: optional_bool(&command.params, "background")?,
        scale: bidi_print_scale(command.params.get("scale"))?,
        paper_width: page.width_inches,
        paper_height: page.height_inches,
        margin_top: margin.top_inches,
        margin_bottom: margin.bottom_inches,
        margin_left: margin.left_inches,
        margin_right: margin.right_inches,
        page_ranges: bidi_print_page_ranges(command.params.get("pageRanges"))?,
        shrink_to_fit: optional_bool(&command.params, "shrinkToFit")?,
        transfer_mode: Some(DevToolsPrintToPdfTransferMode::ReturnAsBase64),
    })
}

#[derive(Default)]
struct BidiPrintMargin {
    top_inches: Option<f64>,
    bottom_inches: Option<f64>,
    left_inches: Option<f64>,
    right_inches: Option<f64>,
}

fn bidi_print_margin(margin: Option<&Value>) -> Result<BidiPrintMargin, BidiError> {
    let Some(margin) = margin else {
        return Ok(BidiPrintMargin::default());
    };
    let Some(margin) = margin.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.print margin must be an object",
        ));
    };
    Ok(BidiPrintMargin {
        top_inches: optional_cm_to_inches(margin, "top", "browsingContext.print margin top")?,
        bottom_inches: optional_cm_to_inches(
            margin,
            "bottom",
            "browsingContext.print margin bottom",
        )?,
        left_inches: optional_cm_to_inches(margin, "left", "browsingContext.print margin left")?,
        right_inches: optional_cm_to_inches(margin, "right", "browsingContext.print margin right")?,
    })
}

#[derive(Default)]
struct BidiPrintPage {
    width_inches: Option<f64>,
    height_inches: Option<f64>,
}

fn bidi_print_page(page: Option<&Value>) -> Result<BidiPrintPage, BidiError> {
    let Some(page) = page else {
        return Ok(BidiPrintPage::default());
    };
    let Some(page) = page.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.print page must be an object",
        ));
    };
    Ok(BidiPrintPage {
        width_inches: optional_print_page_cm_to_inches(
            page,
            "width",
            "browsingContext.print page width",
        )?,
        height_inches: optional_print_page_cm_to_inches(
            page,
            "height",
            "browsingContext.print page height",
        )?,
    })
}

fn optional_cm_to_inches(
    params: &serde_json::Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<Option<f64>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    let Some(value) = value.as_f64() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{label} must be a number"),
        ));
    };
    if !value.is_finite() || value < 0.0 {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{label} must be non-negative"),
        ));
    }
    Ok(Some(value / CENTIMETERS_PER_INCH))
}

fn optional_print_page_cm_to_inches(
    params: &serde_json::Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<Option<f64>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    let Some(value) = value.as_f64() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{label} must be a number"),
        ));
    };
    if !value.is_finite() || value < MIN_PRINT_PAGE_SIZE_CM {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{label} must be at least {MIN_PRINT_PAGE_SIZE_CM:.6} cm"),
        ));
    }
    Ok(Some(value / CENTIMETERS_PER_INCH))
}

fn bidi_print_orientation(orientation: Option<&Value>) -> Result<Option<bool>, BidiError> {
    let Some(orientation) = orientation else {
        return Ok(None);
    };
    let Some(orientation) = orientation.as_str() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.print orientation must be portrait or landscape",
        ));
    };
    match orientation {
        "portrait" => Ok(Some(false)),
        "landscape" => Ok(Some(true)),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.print orientation must be portrait or landscape",
        )),
    }
}

fn bidi_print_scale(scale: Option<&Value>) -> Result<Option<f64>, BidiError> {
    let Some(scale) = scale else {
        return Ok(None);
    };
    let Some(scale) = scale.as_f64() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.print scale must be a number",
        ));
    };
    if !scale.is_finite() || !(0.1..=2.0).contains(&scale) {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.print scale must be between 0.1 and 2.0",
        ));
    }
    Ok(Some(scale))
}

fn bidi_print_page_ranges(page_ranges: Option<&Value>) -> Result<Option<String>, BidiError> {
    let Some(page_ranges) = page_ranges else {
        return Ok(None);
    };
    let Some(page_ranges) = page_ranges.as_array() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.print pageRanges must be an array",
        ));
    };
    let mut ranges = Vec::with_capacity(page_ranges.len());
    for range in page_ranges {
        if let Some(page) = range.as_u64() {
            if page == 0 {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "browsingContext.print pageRanges entries must be positive",
                ));
            }
            ranges.push(page.to_string());
            continue;
        }
        let Some(range) = range.as_str() else {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "browsingContext.print pageRanges entries must be strings or positive integers",
            ));
        };
        if !valid_bidi_print_page_range(range) {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "browsingContext.print pageRanges entry is invalid",
            ));
        }
        ranges.push(range.to_owned());
    }
    Ok(Some(ranges.join(",")))
}

fn valid_bidi_print_page_range(range: &str) -> bool {
    if range.is_empty() {
        return false;
    }
    let Some((start, end)) = range.split_once('-') else {
        return range.parse::<u64>().is_ok_and(|page| page > 0);
    };
    if start.is_empty() && end.is_empty() {
        return false;
    }
    let start = if start.is_empty() {
        None
    } else {
        match start.parse::<u64>() {
            Ok(value) if value > 0 => Some(value),
            _ => return false,
        }
    };
    let end = if end.is_empty() {
        None
    } else {
        match end.parse::<u64>() {
            Ok(value) if value > 0 => Some(value),
            _ => return false,
        }
    };
    match (start, end) {
        (Some(start), Some(end)) => start <= end,
        _ => true,
    }
}

fn bidi_set_viewport_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsSetViewportCommand, BidiError> {
    let has_context = command.params.get("context").is_some();
    let has_user_contexts = command.params.get("userContexts").is_some();
    if has_context == has_user_contexts {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.setViewport must specify exactly one of context or userContexts",
        ));
    }
    if has_user_contexts {
        let user_contexts = optional_non_empty_string_array(&command.params, "userContexts")?
            .expect("presence checked");
        return Ok(DevToolsSetViewportCommand {
            context: context.command_context(None),
            browser_context_ids: user_contexts
                .into_iter()
                .map(DevToolsBrowserContextId::from)
                .collect(),
            viewport: bidi_viewport_setting(command.params.get("viewport"))?,
            device_pixel_ratio: bidi_device_pixel_ratio_setting(
                command.params.get("devicePixelRatio"),
            )?,
            screen_width: None,
            screen_height: None,
        });
    }
    let target_id = required_string(&command.params, "context")?;
    Ok(DevToolsSetViewportCommand {
        context: context.command_context(Some(DevToolsTargetId::from(target_id))),
        browser_context_ids: Vec::new(),
        viewport: bidi_viewport_setting(command.params.get("viewport"))?,
        device_pixel_ratio: bidi_device_pixel_ratio_setting(
            command.params.get("devicePixelRatio"),
        )?,
        screen_width: None,
        screen_height: None,
    })
}

fn bidi_set_user_agent_override_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsSetUserAgentOverrideCommand, BidiError> {
    let scope = bidi_emulation_context_scope(command, false)?;
    let user_agent = required_nullable_string(&command.params, "userAgent")?;
    if user_agent.as_deref() == Some("") {
        return Err(BidiError::new(
            BidiErrorCode::UnsupportedOperation,
            "empty user agent string is not supported",
        ));
    }
    Ok(DevToolsSetUserAgentOverrideCommand {
        context: context.command_context(None),
        target_ids: scope.target_ids,
        browser_context_ids: scope.browser_context_ids,
        user_agent,
    })
}

fn bidi_set_locale_override_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsSetLocaleOverrideCommand, BidiError> {
    let scope = bidi_emulation_context_scope(command, true)?;
    let locale = required_nullable_string(&command.params, "locale")?;
    if let Some(locale) = locale.as_deref() {
        validate_bidi_locale(locale)?;
    }
    Ok(DevToolsSetLocaleOverrideCommand {
        context: context.command_context(None),
        target_ids: scope.target_ids,
        browser_context_ids: scope.browser_context_ids,
        locale,
    })
}

fn bidi_set_timezone_override_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsSetTimezoneOverrideCommand, BidiError> {
    let scope = bidi_emulation_context_scope(command, true)?;
    let timezone = required_nullable_string(&command.params, "timezone")?
        .map(normalized_bidi_timezone)
        .transpose()?;
    Ok(DevToolsSetTimezoneOverrideCommand {
        context: context.command_context(None),
        target_ids: scope.target_ids,
        browser_context_ids: scope.browser_context_ids,
        timezone,
    })
}

fn bidi_set_geolocation_override_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsSetGeolocationOverrideCommand, BidiError> {
    let scope = bidi_emulation_context_scope(command, false)?;
    let has_coordinates = command.params.get("coordinates").is_some();
    let has_error = command.params.get("error").is_some();
    if has_coordinates == has_error {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "emulation.setGeolocationOverride must specify exactly one of coordinates or error",
        ));
    }
    let override_state = if let Some(coordinates) = command.params.get("coordinates") {
        bidi_geolocation_coordinates(coordinates)?.map(DevToolsGeolocationOverrideState::Position)
    } else {
        bidi_geolocation_error(command.params.get("error").expect("presence checked"))?;
        Some(DevToolsGeolocationOverrideState::PositionUnavailable)
    };
    Ok(DevToolsSetGeolocationOverrideCommand {
        context: context.command_context(None),
        target_ids: scope.target_ids,
        browser_context_ids: scope.browser_context_ids,
        override_state,
    })
}

fn bidi_geolocation_coordinates(
    value: &Value,
) -> Result<Option<DevToolsGeolocationOverride>, BidiError> {
    if value.is_null() {
        return Ok(None);
    }
    let Some(coordinates) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "coordinates must be an object or null",
        ));
    };
    let latitude = required_object_finite_number(coordinates, "latitude")?;
    if !(-90.0..=90.0).contains(&latitude) {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "latitude is out of range",
        ));
    }
    let longitude = required_object_finite_number(coordinates, "longitude")?;
    if !(-180.0..=180.0).contains(&longitude) {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "longitude is out of range",
        ));
    }
    let accuracy = match coordinates.get("accuracy") {
        None => 1.0,
        Some(value) => {
            let value = finite_number_value(value, "accuracy")?;
            if value < 0.0 {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "accuracy must be non-negative",
                ));
            }
            value
        }
    };
    let altitude = optional_nullable_object_finite_number(coordinates, "altitude")?;
    let altitude_accuracy =
        optional_nullable_object_finite_number(coordinates, "altitudeAccuracy")?;
    if let Some(altitude_accuracy) = altitude_accuracy {
        if altitude.is_none() {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "altitudeAccuracy requires altitude",
            ));
        }
        if altitude_accuracy < 0.0 {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "altitudeAccuracy must be non-negative",
            ));
        }
    }
    let heading = optional_nullable_object_finite_number(coordinates, "heading")?;
    if let Some(heading) = heading
        && !(0.0..360.0).contains(&heading)
    {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "heading is out of range",
        ));
    }
    let speed = optional_nullable_object_finite_number(coordinates, "speed")?;
    if let Some(speed) = speed
        && speed < 0.0
    {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "speed must be non-negative",
        ));
    }
    Ok(Some(DevToolsGeolocationOverride {
        latitude,
        longitude,
        accuracy,
        altitude,
        altitude_accuracy,
        heading,
        speed,
    }))
}

fn bidi_geolocation_error(value: &Value) -> Result<(), BidiError> {
    let Some(error) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "error must be an object",
        ));
    };
    match error.get("type").and_then(Value::as_str) {
        Some("positionUnavailable") => Ok(()),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "error.type must be positionUnavailable",
        )),
    }
}

fn bidi_set_network_conditions_command(
    command: &BidiCommand,
    context: &BidiDevToolsCommandContext,
) -> Result<DevToolsSetNetworkConditionsCommand, BidiError> {
    let scope = bidi_emulation_context_scope(command, false)?;
    Ok(DevToolsSetNetworkConditionsCommand {
        context: context.command_context(None),
        target_ids: scope.target_ids,
        browser_context_ids: scope.browser_context_ids,
        network_conditions: bidi_network_conditions(command.params.get("networkConditions"))?,
    })
}

fn bidi_network_conditions(
    network_conditions: Option<&Value>,
) -> Result<Option<DevToolsNetworkConditions>, BidiError> {
    let Some(network_conditions) = network_conditions else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "networkConditions must be an object or null",
        ));
    };
    if network_conditions.is_null() {
        return Ok(None);
    }
    let Some(network_conditions) = network_conditions.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "networkConditions must be an object or null",
        ));
    };
    if network_conditions.len() != 1 {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "networkConditions must only specify type",
        ));
    }
    match network_conditions.get("type").and_then(Value::as_str) {
        Some("offline") => Ok(Some(DevToolsNetworkConditions::offline())),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "networkConditions type must be offline",
        )),
    }
}

struct BidiEmulationContextScope {
    target_ids: Vec<DevToolsTargetId>,
    browser_context_ids: Vec<DevToolsBrowserContextId>,
}

fn bidi_emulation_context_scope(
    command: &BidiCommand,
    require_scope: bool,
) -> Result<BidiEmulationContextScope, BidiError> {
    let has_contexts = command.params.get("contexts").is_some();
    let has_user_contexts = command.params.get("userContexts").is_some();
    if has_contexts && has_user_contexts {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!(
                "{} contexts and userContexts are mutually exclusive",
                command.method
            ),
        ));
    }
    if require_scope && !has_contexts && !has_user_contexts {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{} must specify contexts or userContexts", command.method),
        ));
    }
    let target_ids = optional_non_empty_string_array(&command.params, "contexts")?
        .unwrap_or_default()
        .into_iter()
        .map(DevToolsTargetId::from)
        .collect();
    let browser_context_ids = optional_non_empty_string_array(&command.params, "userContexts")?
        .unwrap_or_default()
        .into_iter()
        .map(DevToolsBrowserContextId::from)
        .collect();
    Ok(BidiEmulationContextScope {
        target_ids,
        browser_context_ids,
    })
}

fn bidi_viewport_setting(viewport: Option<&Value>) -> Result<DevToolsViewportSetting, BidiError> {
    let Some(viewport) = viewport else {
        return Ok(DevToolsViewportSetting::Unchanged);
    };
    if viewport.is_null() {
        return Ok(DevToolsViewportSetting::Default);
    }
    let Some(viewport) = viewport.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.setViewport viewport must be an object or null",
        ));
    };
    let width = required_object_u32(
        viewport,
        "width",
        "browsingContext.setViewport viewport width",
    )?;
    let height = required_object_u32(
        viewport,
        "height",
        "browsingContext.setViewport viewport height",
    )?;
    Ok(DevToolsViewportSetting::Dimensions { width, height })
}

fn bidi_device_pixel_ratio_setting(
    device_pixel_ratio: Option<&Value>,
) -> Result<DevToolsDevicePixelRatioSetting, BidiError> {
    let Some(device_pixel_ratio) = device_pixel_ratio else {
        return Ok(DevToolsDevicePixelRatioSetting::Unchanged);
    };
    if device_pixel_ratio.is_null() {
        return Ok(DevToolsDevicePixelRatioSetting::Default);
    }
    let Some(device_pixel_ratio) = device_pixel_ratio.as_f64() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.setViewport devicePixelRatio must be a positive number or null",
        ));
    };
    if !device_pixel_ratio.is_finite() || device_pixel_ratio <= 0.0 {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "browsingContext.setViewport devicePixelRatio must be positive",
        ));
    }
    Ok(DevToolsDevicePixelRatioSetting::Scale(device_pixel_ratio))
}

fn bidi_result_ownership(ownership: Option<&Value>) -> Result<DevToolsResultOwnership, BidiError> {
    let Some(ownership) = ownership else {
        return Ok(DevToolsResultOwnership::None);
    };
    let Some(ownership) = ownership.as_str() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "script resultOwnership must be none or root",
        ));
    };
    match ownership {
        "none" => Ok(DevToolsResultOwnership::None),
        "root" => Ok(DevToolsResultOwnership::Root),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "script resultOwnership must be none or root",
        )),
    }
}

fn required_string<'a>(params: &'a Value, field: &str) -> Result<&'a str, BidiError> {
    params.get(field).and_then(Value::as_str).ok_or_else(|| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a string"),
        )
    })
}

fn required_nullable_string(params: &Value, field: &str) -> Result<Option<String>, BidiError> {
    let Some(value) = params.get(field) else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a string or null"),
        ));
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(|value| Some(value.to_owned()))
        .ok_or_else(|| {
            BidiError::new(
                BidiErrorCode::InvalidArgument,
                format!("{field} must be a string or null"),
            )
        })
}

fn validate_bidi_locale(locale: &str) -> Result<(), BidiError> {
    if locale.is_empty() || !locale.is_ascii() {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("invalid locale {locale}"),
        ));
    }
    let subtags = locale.split('-').collect::<Vec<_>>();
    if subtags.iter().any(|subtag| subtag.is_empty()) {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("invalid locale {locale}"),
        ));
    }
    let Some(language) = subtags.first() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("invalid locale {locale}"),
        ));
    };
    if !(2..=3).contains(&language.len())
        || !language.chars().all(|value| value.is_ascii_alphabetic())
    {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("invalid locale {locale}"),
        ));
    }
    let mut index = 1;
    if subtags.get(index).is_some_and(|subtag| {
        subtag.len() == 4 && subtag.chars().all(|value| value.is_ascii_alphabetic())
    }) {
        index += 1;
    }
    if subtags.get(index).is_some_and(|subtag| {
        (subtag.len() == 2 && subtag.chars().all(|value| value.is_ascii_alphabetic()))
            || (subtag.len() == 3 && subtag.chars().all(|value| value.is_ascii_digit()))
    }) {
        index += 1;
    }
    while let Some(subtag) = subtags.get(index) {
        if subtag.len() == 1 {
            let singleton = subtag.as_bytes()[0].to_ascii_lowercase();
            if singleton == b'x' || !subtag.chars().all(|value| value.is_ascii_alphanumeric()) {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    format!("invalid locale {locale}"),
                ));
            }
            index += 1;
            let extension_start = index;
            while let Some(extension) = subtags.get(index) {
                if extension.len() == 1 {
                    break;
                }
                if !(2..=8).contains(&extension.len())
                    || !extension.chars().all(|value| value.is_ascii_alphanumeric())
                {
                    return Err(BidiError::new(
                        BidiErrorCode::InvalidArgument,
                        format!("invalid locale {locale}"),
                    ));
                }
                index += 1;
            }
            if index == extension_start {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    format!("invalid locale {locale}"),
                ));
            }
            continue;
        }
        let is_variant = (5..=8).contains(&subtag.len())
            && subtag.chars().all(|value| value.is_ascii_alphanumeric())
            || (subtag.len() == 4
                && subtag
                    .chars()
                    .next()
                    .is_some_and(|value| value.is_ascii_digit())
                && subtag.chars().all(|value| value.is_ascii_alphanumeric()));
        if !is_variant {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                format!("invalid locale {locale}"),
            ));
        }
        index += 1;
    }
    Ok(())
}

fn normalized_bidi_timezone(timezone: String) -> Result<String, BidiError> {
    if timezone.is_empty() {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "invalid timezone",
        ));
    }
    if is_timezone_offset_string(&timezone) {
        return Ok(format!("GMT{timezone}"));
    }
    if timezone.starts_with("GMT+")
        || timezone.starts_with("GMT-")
        || timezone.starts_with("UTC+")
        || timezone.starts_with("UTC-")
    {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "invalid timezone",
        ));
    }
    if !timezone.chars().all(is_timezone_name_char) {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "invalid timezone",
        ));
    }
    // This adapter only normalizes BiDi syntax. The process environment
    // controller validates names against the same ICU database that Date/Intl
    // use. System zoneinfo and region heuristics can disagree with that data.
    Ok(timezone)
}

fn is_timezone_name_char(value: char) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, '_' | '-' | '+' | '/')
}

fn is_timezone_offset_string(timezone: &str) -> bool {
    let bytes = timezone.as_bytes();
    bytes.len() == 6
        && matches!(bytes[0], b'+' | b'-')
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3] == b':'
        && bytes[4].is_ascii_digit()
        && bytes[5].is_ascii_digit()
}

fn required_safe_integer(params: &Value, field: &str) -> Result<i64, BidiError> {
    let Some(value) = params.get(field) else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a safe integer"),
        ));
    };
    let Some(number) = value.as_number() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a safe integer"),
        ));
    };
    let Some(value) = number
        .as_i64()
        .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
    else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a safe integer"),
        ));
    };
    if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a safe integer"),
        ));
    }
    Ok(value)
}

fn optional_uint(params: &Value, field: &str) -> Result<Option<u32>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    let Some(value) = value.as_u64().and_then(|value| u32::try_from(value).ok()) else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a uint"),
        ));
    };
    Ok(Some(value))
}

fn optional_int(params: &Value, field: &str) -> Result<Option<i32>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    let Some(value) = value
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .or_else(|| value.as_u64().and_then(|value| i32::try_from(value).ok()))
    else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be an int"),
        ));
    };
    Ok(Some(value))
}

fn optional_bool(params: &Value, field: &str) -> Result<Option<bool>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    required_bool_value(value, field).map(Some)
}

fn validate_optional_bool(params: &Value, field: &str) -> Result<(), BidiError> {
    if let Some(value) = params.get(field) {
        required_bool_value(value, field)?;
    }
    Ok(())
}

fn required_bool_value(value: &Value, field: &str) -> Result<bool, BidiError> {
    value.as_bool().ok_or_else(|| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a boolean"),
        )
    })
}

fn optional_string<'a>(params: &'a Value, field: &str) -> Result<Option<&'a str>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    value.as_str().map(Some).ok_or_else(|| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a string"),
        )
    })
}

pub(crate) fn required_object_value<'a>(
    params: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a Value, BidiError> {
    params.get(field).ok_or_else(|| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} is required"),
        )
    })
}

pub(crate) fn optional_object_value<'a>(
    params: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Option<&'a Value> {
    params.get(field)
}

pub(crate) fn required_object_string<'a>(
    params: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str, BidiError> {
    params.get(field).and_then(Value::as_str).ok_or_else(|| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a string"),
        )
    })
}

fn required_object_finite_number(
    params: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<f64, BidiError> {
    let value = params.get(field).ok_or_else(|| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a number"),
        )
    })?;
    finite_number_value(value, field)
}

fn optional_nullable_object_finite_number(
    params: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<f64>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    finite_number_value(value, field).map(Some)
}

fn finite_number_value(value: &Value, field: &str) -> Result<f64, BidiError> {
    let Some(value) = value.as_f64() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a number"),
        ));
    };
    if !value.is_finite() {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be finite"),
        ));
    }
    Ok(value)
}

fn required_object_u32(
    params: &serde_json::Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<u32, BidiError> {
    let Some(value) = params.get(field).and_then(Value::as_u64) else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{label} must be a uint"),
        ));
    };
    u32::try_from(value).map_err(|_| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{label} is too large"),
        )
    })
}

pub(crate) fn optional_object_string<'a>(
    params: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<&'a str>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    value.as_str().map(Some).ok_or_else(|| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a string"),
        )
    })
}

pub(crate) fn optional_object_bool(
    params: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<bool>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    value.as_bool().map(Some).ok_or_else(|| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a boolean"),
        )
    })
}

pub(crate) fn optional_object_uint(
    params: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<u64>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    value.as_u64().map(Some).ok_or_else(|| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a uint"),
        )
    })
}

pub(crate) fn required_network_bytes_value(
    value: &Value,
    field: &str,
) -> Result<String, BidiError> {
    let Some(value) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a network bytes value"),
        ));
    };
    let type_name = required_object_string(value, "type")?;
    let raw = required_object_string(value, "value")?;
    match type_name {
        "string" => Ok(raw.to_owned()),
        "base64" => {
            let bytes = BASE64_STANDARD.decode(raw).map_err(|_| {
                BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    format!("{field} base64 value is invalid"),
                )
            })?;
            String::from_utf8(bytes).map_err(|_| {
                BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    format!("{field} base64 value must decode to utf-8"),
                )
            })
        }
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} type must be string or base64"),
        )),
    }
}

fn optional_string_array(params: &Value, field: &str) -> Result<Option<Vec<String>>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    let Some(items) = value.as_array() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be an array"),
        ));
    };
    let mut strings = Vec::with_capacity(items.len());
    for item in items {
        let Some(value) = item.as_str() else {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                format!("{field} entries must be strings"),
            ));
        };
        strings.push(value.to_owned());
    }
    Ok(Some(strings))
}

pub(crate) fn required_non_empty_string_array(
    params: &Value,
    field: &str,
) -> Result<Vec<String>, BidiError> {
    let Some(strings) = optional_string_array(params, field)? else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be an array"),
        ));
    };
    if strings.is_empty() {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must not be empty"),
        ));
    }
    Ok(strings)
}

pub(crate) fn optional_non_empty_string_array(
    params: &Value,
    field: &str,
) -> Result<Option<Vec<String>>, BidiError> {
    let Some(strings) = optional_string_array(params, field)? else {
        return Ok(None);
    };
    if strings.is_empty() {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must not be empty"),
        ));
    }
    Ok(Some(strings))
}

pub(crate) fn required_supported_event_array(
    params: &Value,
    field: &str,
) -> Result<Vec<String>, BidiError> {
    let events = required_non_empty_string_array(params, field)?;
    for event in &events {
        if !is_supported_bidi_subscription_event(event) {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                format!("Unknown event: {event}"),
            ));
        }
    }
    Ok(events)
}

fn is_supported_bidi_subscription_event(event: &str) -> bool {
    matches!(
        event,
        "script"
            | "script.message"
            | "script.realmCreated"
            | "script.realmDestroyed"
            | "browsingContext"
            | "browsingContext.contextCreated"
            | "browsingContext.contextDestroyed"
            | "browsingContext.navigationStarted"
            | "browsingContext.fragmentNavigated"
            | "browsingContext.historyUpdated"
            | "browsingContext.domContentLoaded"
            | "browsingContext.downloadWillBegin"
            | "browsingContext.downloadEnd"
            | "browsingContext.load"
            | "browsingContext.userPromptOpened"
            | "browsingContext.userPromptClosed"
            | "input"
            | "input.fileDialogOpened"
            | "log"
            | "log.entryAdded"
            | "network"
            | "network.beforeRequestSent"
            | "network.responseStarted"
            | "network.authRequired"
            | "network.responseCompleted"
            | "network.fetchError"
    )
}

pub(crate) fn unroll_bidi_events<'a>(events: &'a [String]) -> impl Iterator<Item = String> + 'a {
    events.iter().flat_map(|event| match event.as_str() {
        "script" => vec![
            "script.message".to_owned(),
            "script.realmCreated".to_owned(),
            "script.realmDestroyed".to_owned(),
        ],
        "browsingContext" => vec![
            "browsingContext.contextCreated".to_owned(),
            "browsingContext.contextDestroyed".to_owned(),
            "browsingContext.navigationStarted".to_owned(),
            "browsingContext.fragmentNavigated".to_owned(),
            "browsingContext.historyUpdated".to_owned(),
            "browsingContext.domContentLoaded".to_owned(),
            "browsingContext.downloadWillBegin".to_owned(),
            "browsingContext.downloadEnd".to_owned(),
            "browsingContext.load".to_owned(),
            "browsingContext.userPromptOpened".to_owned(),
            "browsingContext.userPromptClosed".to_owned(),
        ],
        "input" => vec!["input.fileDialogOpened".to_owned()],
        "log" => vec!["log.entryAdded".to_owned()],
        "network" => vec![
            "network.beforeRequestSent".to_owned(),
            "network.responseStarted".to_owned(),
            "network.authRequired".to_owned(),
            "network.responseCompleted".to_owned(),
            "network.fetchError".to_owned(),
        ],
        _ => vec![event.clone()],
    })
}

fn optional_script_argument_array(params: &Value, field: &str) -> Result<Vec<Value>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be an array"),
        ));
    };
    for item in items {
        validate_script_argument_value(item, &format!("{field} entries"))?;
    }
    Ok(items.clone())
}

fn optional_channel_argument_array(params: &Value, field: &str) -> Result<Vec<Value>, BidiError> {
    let Some(value) = params.get(field) else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be an array"),
        ));
    };
    for item in items {
        validate_channel_script_argument_value(item, &format!("{field} entries"))?;
    }
    Ok(items.clone())
}

fn validate_channel_script_argument_value(value: &Value, field: &str) -> Result<(), BidiError> {
    let Some(argument) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be an object"),
        ));
    };
    if argument.get("type").and_then(Value::as_str) != Some("channel") {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} type must be channel"),
        ));
    }
    validate_script_argument_value(value, field)
}

fn validate_optional_script_argument(params: &Value, field: &str) -> Result<(), BidiError> {
    if let Some(value) = params.get(field) {
        validate_script_argument_value(value, field)?;
    }
    Ok(())
}

fn validate_script_argument_value(value: &Value, field: &str) -> Result<(), BidiError> {
    let Some(argument) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be an object"),
        ));
    };

    if let Some(handle) = argument.get("handle")
        && !handle.is_string()
    {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} handle must be a string"),
        ));
    }
    if let Some(shared_id) = argument.get("sharedId")
        && !shared_id.is_string()
    {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} sharedId must be a string"),
        ));
    }
    if argument.get("handle").is_some() || argument.get("sharedId").is_some() {
        return Ok(());
    }

    let Some(type_name) = argument.get("type").and_then(Value::as_str) else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must have a type, handle, or sharedId"),
        ));
    };

    match type_name {
        "undefined" | "null" => return Ok(()),
        "string" | "bigint" | "date" => required_value(argument, field)?.as_str().map(|_| ()),
        "number" => {
            let value = required_value(argument, field)?;
            if value.is_number()
                || matches!(
                    value.as_str(),
                    Some("NaN" | "-0" | "Infinity" | "-Infinity")
                )
            {
                Some(())
            } else {
                None
            }
        }
        "boolean" => required_value(argument, field)?.as_bool().map(|_| ()),
        "array" | "set" => {
            let values = required_value(argument, field)?
                .as_array()
                .ok_or_else(|| script_argument_value_error(field))?;
            for value in values {
                validate_script_argument_value(value, field)?;
            }
            Some(())
        }
        "map" => {
            let entries = required_value(argument, field)?
                .as_array()
                .ok_or_else(|| script_argument_value_error(field))?;
            for entry in entries {
                let pair = entry
                    .as_array()
                    .ok_or_else(|| script_argument_value_error(field))?;
                let [key, value] = pair.as_slice() else {
                    return Err(script_argument_value_error(field));
                };
                if !key.is_string() {
                    validate_script_argument_value(key, field)?;
                }
                validate_script_argument_value(value, field)?;
            }
            Some(())
        }
        "object" => {
            let entries = required_value(argument, field)?
                .as_array()
                .ok_or_else(|| script_argument_value_error(field))?;
            for entry in entries {
                let pair = entry
                    .as_array()
                    .ok_or_else(|| script_argument_value_error(field))?;
                let [key, value] = pair.as_slice() else {
                    return Err(script_argument_value_error(field));
                };
                if !key.is_string() {
                    return Err(script_argument_value_error(field));
                }
                validate_script_argument_value(value, field)?;
            }
            Some(())
        }
        "regexp" => validate_regexp_argument_value(required_value(argument, field)?),
        "channel" => validate_channel_argument_value(required_value(argument, field)?),
        _ => {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                format!("{field} type is not supported"),
            ));
        }
    }
    .ok_or_else(|| script_argument_value_error(field))
}

fn script_argument_value_error(field: &str) -> BidiError {
    BidiError::new(
        BidiErrorCode::InvalidArgument,
        format!("{field} value does not match its type"),
    )
}

fn required_value<'a>(
    argument: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a Value, BidiError> {
    argument.get("value").ok_or_else(|| {
        BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} value is required"),
        )
    })
}

fn validate_regexp_argument_value(value: &Value) -> Option<()> {
    let value = value.as_object()?;
    value.get("pattern")?.as_str()?;
    if let Some(flags) = value.get("flags") {
        flags.as_str()?;
    }
    Some(())
}

fn validate_channel_argument_value(value: &Value) -> Option<()> {
    let value = value.as_object()?;
    value.get("channel")?.as_str()?;
    if let Some(ownership) = value.get("ownership") {
        matches!(ownership.as_str(), Some("none" | "root")).then_some(())?;
    }
    if let Some(serialization_options) = value.get("serializationOptions") {
        validate_serialization_options(serialization_options).ok()?;
    }
    Some(())
}

fn validate_optional_serialization_options(params: &Value) -> Result<(), BidiError> {
    if let Some(value) = params.get("serializationOptions") {
        validate_serialization_options(value)?;
    }
    Ok(())
}

fn bidi_serialization_options(
    params: &Value,
) -> Result<Option<DevToolsSerializationOptions>, BidiError> {
    let Some(value) = params.get("serializationOptions") else {
        return Ok(None);
    };
    let Some(options) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "serializationOptions must be an object",
        ));
    };
    let max_object_depth = match options.get("maxObjectDepth") {
        Some(Value::Null) | None => None,
        Some(value) => Some(value.as_u64().ok_or_else(|| {
            BidiError::new(
                BidiErrorCode::InvalidArgument,
                "serializationOptions.maxObjectDepth must be a uint or null",
            )
        })?),
    };
    let max_dom_depth = match options.get("maxDomDepth") {
        Some(Value::Null) | None => None,
        Some(value) => Some(value.as_u64().ok_or_else(|| {
            BidiError::new(
                BidiErrorCode::InvalidArgument,
                "serializationOptions.maxDomDepth must be a uint or null",
            )
        })?),
    };
    let include_shadow_tree = match options.get("includeShadowTree") {
        None => None,
        Some(Value::String(value)) if matches!(value.as_str(), "none" | "open" | "all") => {
            Some(value.clone())
        }
        Some(_) => {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "serializationOptions.includeShadowTree must be none, open, or all",
            ));
        }
    };
    Ok(Some(DevToolsSerializationOptions {
        max_object_depth,
        max_dom_depth,
        include_shadow_tree,
    }))
}

fn bidi_script_serialization_options(
    params: &Value,
) -> Result<DevToolsSerializationOptions, BidiError> {
    Ok(bidi_serialization_options(params)?
        .unwrap_or_else(bidi_default_script_serialization_options))
}

fn bidi_default_script_serialization_options() -> DevToolsSerializationOptions {
    DevToolsSerializationOptions {
        max_object_depth: Some(2),
        max_dom_depth: Some(1),
        include_shadow_tree: None,
    }
}

fn validate_serialization_options(value: &Value) -> Result<(), BidiError> {
    let Some(options) = value.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "serializationOptions must be an object",
        ));
    };
    if let Some(value) = options.get("maxDomDepth") {
        validate_nullable_uint(value, "serializationOptions.maxDomDepth")?;
    }
    if let Some(value) = options.get("maxObjectDepth") {
        validate_nullable_uint(value, "serializationOptions.maxObjectDepth")?;
    }
    if let Some(value) = options.get("includeShadowTree")
        && !matches!(value.as_str(), Some("none" | "open" | "all"))
    {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "serializationOptions.includeShadowTree must be none, open, or all",
        ));
    }
    Ok(())
}

fn validate_nullable_uint(value: &Value, field: &str) -> Result<(), BidiError> {
    if value.is_null() || value.as_u64().is_some() {
        Ok(())
    } else {
        Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            format!("{field} must be a uint or null"),
        ))
    }
}

fn is_bidi_realm_type(realm_type: &str) -> bool {
    matches!(
        realm_type,
        "window"
            | "dedicated-worker"
            | "shared-worker"
            | "service-worker"
            | "worker"
            | "worklet"
            | "paint-worklet"
            | "audio-worklet"
    )
}
