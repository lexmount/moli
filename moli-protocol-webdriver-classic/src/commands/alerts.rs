use moli_protocol::automation::{
    AutomationCommand, DevToolsGetJavaScriptDialogCommand, DevToolsHandleJavaScriptDialogCommand,
    DevToolsSetJavaScriptDialogPromptTextCommand,
};
use serde_json::Value;

use crate::{ClassicDevToolsCommandContext, ClassicError};

use super::parsing::required_string;

pub fn alert_text_command(context: &ClassicDevToolsCommandContext) -> AutomationCommand {
    AutomationCommand::GetJavaScriptDialog(DevToolsGetJavaScriptDialogCommand {
        context: context.command_context(),
    })
}

pub fn alert_handle_command(
    context: &ClassicDevToolsCommandContext,
    accept: bool,
) -> AutomationCommand {
    AutomationCommand::HandleJavaScriptDialog(DevToolsHandleJavaScriptDialogCommand {
        context: context.command_context(),
        accept,
        prompt_text: String::new(),
    })
}

pub fn alert_send_text_command(
    context: &ClassicDevToolsCommandContext,
    params: &Value,
) -> Result<AutomationCommand, ClassicError> {
    Ok(AutomationCommand::SetJavaScriptDialogPromptText(
        DevToolsSetJavaScriptDialogPromptTextCommand {
            context: context.command_context(),
            prompt_text: required_string(params, "text")?.to_owned(),
        },
    ))
}
