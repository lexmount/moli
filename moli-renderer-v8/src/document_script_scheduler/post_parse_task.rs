use crate::{
    frame_owner_model::MainDocumentScriptLoadDelayLease,
    planning::{PreparedScript, SharedScriptSourceLoad},
    types::SharedNavigationResponseResult,
};

pub(super) enum PostParseDocumentScriptTask {
    AsyncScript(Box<PostParseAsyncScriptTask>),
}

pub(super) enum PostParseAsyncScriptTask {
    Ready {
        script: PreparedScript,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    },
    WaitingForSource {
        script: PreparedScript,
        source_load: SharedScriptSourceLoad,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    },
    Failure {
        script: PreparedScript,
        error: String,
        source_network_result: Option<SharedNavigationResponseResult>,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    },
}

impl PostParseDocumentScriptTask {
    pub(super) fn async_script(
        script: PreparedScript,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) -> Self {
        Self::AsyncScript(Box::new(PostParseAsyncScriptTask::Ready {
            script,
            load_delay_binding,
        }))
    }

    pub(super) fn async_script_waiting_for_source(
        script: PreparedScript,
        source_load: SharedScriptSourceLoad,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) -> Self {
        Self::AsyncScript(Box::new(PostParseAsyncScriptTask::WaitingForSource {
            script,
            source_load,
            load_delay_binding,
        }))
    }

    pub(super) fn async_script_load_failure(
        script: PreparedScript,
        error: String,
        source_network_result: Option<SharedNavigationResponseResult>,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) -> Self {
        Self::AsyncScript(Box::new(PostParseAsyncScriptTask::Failure {
            script,
            error,
            source_network_result,
            load_delay_binding,
        }))
    }

    #[cfg(test)]
    pub(super) fn as_script(&self) -> Option<&PreparedScript> {
        match self {
            Self::AsyncScript(task) => Some(task.script()),
        }
    }

    #[cfg(test)]
    pub(super) fn is_waiting_for_source_load(&self) -> bool {
        matches!(
            self,
            Self::AsyncScript(task) if matches!(task.as_ref(), PostParseAsyncScriptTask::WaitingForSource { .. })
        )
    }

    #[cfg(test)]
    pub(super) fn is_async_script_failure(&self) -> bool {
        matches!(
            self,
            Self::AsyncScript(task) if matches!(task.as_ref(), PostParseAsyncScriptTask::Failure { .. })
        )
    }
}

impl PostParseAsyncScriptTask {
    #[cfg(test)]
    fn script(&self) -> &PreparedScript {
        match self {
            Self::Ready { script, .. }
            | Self::WaitingForSource { script, .. }
            | Self::Failure { script, .. } => script,
        }
    }
}
