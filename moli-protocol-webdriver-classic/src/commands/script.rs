use moli_protocol::automation::{
    AutomationCommand, DevToolsCallFunctionCommand, DevToolsResultOwnership,
};
use serde_json::Value;

use crate::{ClassicDevToolsCommandContext, ClassicError};

use super::parsing::{classic_script_arguments, required_string};

pub fn execute_sync_command(
    context: &ClassicDevToolsCommandContext,
    params: &Value,
) -> Result<AutomationCommand, ClassicError> {
    let script = required_string(params, "script")?;
    Ok(AutomationCommand::CallFunction(
        DevToolsCallFunctionCommand {
            context: context.command_context(),
            realm_id: None,
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: format!("async function() {{\n{script}\n}}"),
            arguments: classic_script_arguments(params)?,
            await_promise: true,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::None,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        },
    ))
}

pub fn execute_async_command(
    context: &ClassicDevToolsCommandContext,
    params: &Value,
) -> Result<AutomationCommand, ClassicError> {
    let script = required_string(params, "script")?;
    Ok(AutomationCommand::CallFunction(
        DevToolsCallFunctionCommand {
            context: context.command_context(),
            realm_id: None,
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: format!(
                "function() {{\n\
             const __moliUserFunction = async function() {{\n\
             {script}\n\
             }};\n\
             return new Promise((resolve, reject) => {{\n\
             const __moliArgs = Array.prototype.slice.call(arguments);\n\
             let __moliDone = false;\n\
             const __moliCallback = function(value) {{\n\
             if (__moliDone) return;\n\
             __moliDone = true;\n\
             resolve(value);\n\
             }};\n\
             __moliArgs.push(__moliCallback);\n\
             try {{\n\
             const __moliScriptResult = __moliUserFunction.apply(this, __moliArgs);\n\
             if (__moliScriptResult && typeof __moliScriptResult.then === 'function') {{\n\
             Promise.resolve(__moliScriptResult).catch((error) => {{\n\
             if (!__moliDone) {{\n\
             __moliDone = true;\n\
             reject(error);\n\
             }}\n\
             }});\n\
             }}\n\
             }} catch (error) {{\n\
             if (!__moliDone) {{\n\
             __moliDone = true;\n\
             reject(error);\n\
             }}\n\
             }}\n\
             }});\n\
             }}"
            ),
            arguments: classic_script_arguments(params)?,
            await_promise: true,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::None,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        },
    ))
}
