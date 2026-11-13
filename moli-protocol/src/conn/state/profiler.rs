use serde_json::{Map, Value};

use super::{ProfilerAction, inspector::InspectorCommandDispatch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfilerInspectorCommand {
    Enable,
    Disable,
    SetSamplingInterval,
    Start,
    Stop,
    StartPreciseCoverage,
    TakePreciseCoverage,
    StopPreciseCoverage,
    GetBestEffortCoverage,
}

impl ProfilerInspectorCommand {
    pub(crate) fn from_action(action: ProfilerAction) -> Self {
        match action {
            ProfilerAction::Enable => Self::Enable,
            ProfilerAction::Disable => Self::Disable,
            ProfilerAction::SetSamplingInterval => Self::SetSamplingInterval,
            ProfilerAction::Start => Self::Start,
            ProfilerAction::Stop => Self::Stop,
            ProfilerAction::StartPreciseCoverage => Self::StartPreciseCoverage,
            ProfilerAction::TakePreciseCoverage => Self::TakePreciseCoverage,
            ProfilerAction::StopPreciseCoverage => Self::StopPreciseCoverage,
            ProfilerAction::GetBestEffortCoverage => Self::GetBestEffortCoverage,
        }
    }

    pub(crate) fn protocol_method(self) -> &'static str {
        match self {
            Self::Enable => "Profiler.enable",
            Self::Disable => "Profiler.disable",
            Self::SetSamplingInterval => "Profiler.setSamplingInterval",
            Self::Start => "Profiler.start",
            Self::Stop => "Profiler.stop",
            Self::StartPreciseCoverage => "Profiler.startPreciseCoverage",
            Self::TakePreciseCoverage => "Profiler.takePreciseCoverage",
            Self::StopPreciseCoverage => "Profiler.stopPreciseCoverage",
            Self::GetBestEffortCoverage => "Profiler.getBestEffortCoverage",
        }
    }

    pub(crate) fn runtime_dispatch(
        self,
        command_id: Option<u64>,
        params: Option<&Map<String, Value>>,
    ) -> InspectorCommandDispatch {
        InspectorCommandDispatch::new(
            self.protocol_method(),
            build_profiler_inspector_command_json(command_id, self.protocol_method(), params),
        )
    }
}

fn build_profiler_inspector_command_json(
    command_id: Option<u64>,
    method: &'static str,
    params: Option<&Map<String, Value>>,
) -> String {
    let mut message = Map::new();
    if let Some(command_id) = command_id {
        message.insert("id".to_owned(), Value::from(command_id));
    }
    message.insert("method".to_owned(), Value::String(method.to_owned()));
    if let Some(params) = params {
        message.insert("params".to_owned(), Value::Object(params.clone()));
    }
    Value::Object(message).to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{ProfilerAction, ProfilerInspectorCommand};

    #[test]
    fn profiler_inspector_command_owns_protocol_method() {
        let start_precise_command =
            ProfilerInspectorCommand::from_action(ProfilerAction::StartPreciseCoverage);
        let params = json!({ "detailed": true });
        assert_eq!(
            start_precise_command.protocol_method(),
            "Profiler.startPreciseCoverage"
        );
        assert_eq!(
            start_precise_command
                .runtime_dispatch(Some(1), params.as_object())
                .protocol_method(),
            "Profiler.startPreciseCoverage"
        );
    }

    #[test]
    fn profiler_runtime_dispatch_owns_v8_inspector_message_shape() {
        let command = ProfilerInspectorCommand::from_action(ProfilerAction::SetSamplingInterval);
        let params = json!({"interval": 100, "unknownIgnoredByV8": true});
        let dispatch = command.runtime_dispatch(Some(42), params.as_object());
        let message: Value = serde_json::from_str(&dispatch.into_inspector_json())
            .expect("profiler dispatch should build valid inspector JSON");

        assert_eq!(
            message,
            json!({
                "id": 42,
                "method": "Profiler.setSamplingInterval",
                "params": {
                    "interval": 100,
                    "unknownIgnoredByV8": true
                }
            })
        );
    }
}
