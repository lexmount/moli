use moli_protocol::automation::{
    AutomationCommand, DevToolsEvaluateScriptCommand, DevToolsGetNavigationHistoryCommand,
    DevToolsGetNavigationHistoryResult, DevToolsGetOuterHtmlCommand, DevToolsGetTargetsCommand,
    DevToolsHistoryTraversalDestination, DevToolsNavigateCommand, DevToolsNavigationWait,
    DevToolsReloadCommand, DevToolsResultOwnership, DevToolsTargetId,
    DevToolsTraverseHistoryCommand,
};
use serde_json::Value;
use url::Url;

use crate::{ClassicDevToolsCommandContext, ClassicError, ClassicErrorCode};

use super::parsing::required_string;

pub fn navigate_command(
    context: &ClassicDevToolsCommandContext,
    params: &Value,
    wait: DevToolsNavigationWait,
) -> Result<AutomationCommand, ClassicError> {
    let url = required_string(params, "url")?;
    if Url::parse(url).is_err() {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "url must be a valid absolute URL",
        ));
    }
    Ok(AutomationCommand::Navigate(DevToolsNavigateCommand {
        context: context.command_context(),
        url: url.to_owned(),
        referrer: None,
        wait,
    }))
}

pub fn current_url_command(context: &ClassicDevToolsCommandContext) -> AutomationCommand {
    AutomationCommand::GetTargets(DevToolsGetTargetsCommand {
        context: context.command_context(),
        root: context.target_id.as_deref().map(DevToolsTargetId::from),
        max_depth: None,
        filter: None,
    })
}

pub fn title_command(context: &ClassicDevToolsCommandContext) -> AutomationCommand {
    AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        expression: "document.title".to_owned(),
        await_promise: true,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn page_source_command(context: &ClassicDevToolsCommandContext) -> AutomationCommand {
    AutomationCommand::GetOuterHtml(DevToolsGetOuterHtmlCommand {
        context: context.command_context(),
        reference: None,
        include_shadow_dom: false,
    })
}

pub fn refresh_command(
    context: &ClassicDevToolsCommandContext,
    wait: DevToolsNavigationWait,
) -> AutomationCommand {
    AutomationCommand::Reload(DevToolsReloadCommand {
        context: context.command_context(),
        ignore_cache: false,
        script_to_evaluate_on_load: None,
        wait,
    })
}

pub fn navigation_history_command(context: &ClassicDevToolsCommandContext) -> AutomationCommand {
    AutomationCommand::GetNavigationHistory(DevToolsGetNavigationHistoryCommand {
        context: context.command_context(),
    })
}

pub fn history_traversal_entry(
    history: &DevToolsGetNavigationHistoryResult,
    delta: i32,
) -> Option<(i32, String)> {
    let target_index = history.current_index as i64 + i64::from(delta);
    if target_index < 0 || target_index >= history.entries.len() as i64 {
        return None;
    }
    let entry = &history.entries[target_index as usize];
    Some((entry.id, entry.url.clone()))
}

pub fn traverse_history_command(
    context: &ClassicDevToolsCommandContext,
    entry_id: i32,
    url: impl Into<String>,
    wait: DevToolsNavigationWait,
) -> AutomationCommand {
    AutomationCommand::TraverseHistory(DevToolsTraverseHistoryCommand {
        context: context.command_context(),
        destination: DevToolsHistoryTraversalDestination::Entry {
            entry_id,
            url: url.into(),
        },
        wait,
    })
}
