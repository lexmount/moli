use crate::{
    dom::NodeId,
    frame_owner_model::MainDocumentScriptLoadDelayLease,
    planning::{PreparedScript, PreparedScriptSourceLoadOutcome},
    types::SharedNavigationResponseResult,
};

#[derive(Debug)]
pub(crate) enum ParseTimeDocumentScriptEvent {
    ReadyTask(Box<ParseTimeDocumentScriptTask>),
    AsyncCompletion(ParseTimeAsyncCompletion),
}

#[derive(Debug)]
pub(crate) struct ParseTimeAsyncCompletion {
    node_id: NodeId,
    outcome: PreparedScriptSourceLoadOutcome,
}

#[derive(Debug)]
pub(crate) enum ParseTimeDocumentScriptTask {
    ClassicAsyncScript(ParseTimeAsyncScript),
    AsyncScriptFailure(ParseTimeAsyncScriptFailure),
}

#[derive(Debug)]
pub(crate) struct ParseTimeAsyncScript {
    script: PreparedScript,
    source_network_result: Option<SharedNavigationResponseResult>,
    load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
}

#[derive(Debug)]
pub(crate) struct ParseTimeAsyncScriptFailure {
    script: ParseTimeAsyncScript,
    error: String,
    source_network_result: Option<SharedNavigationResponseResult>,
}

impl ParseTimeDocumentScriptEvent {
    pub(crate) fn ready_task(task: ParseTimeDocumentScriptTask) -> Self {
        Self::ReadyTask(Box::new(task))
    }

    pub(crate) fn async_completion(
        node_id: NodeId,
        outcome: PreparedScriptSourceLoadOutcome,
    ) -> Self {
        Self::AsyncCompletion(ParseTimeAsyncCompletion { node_id, outcome })
    }
}

impl ParseTimeAsyncCompletion {
    pub(crate) fn into_parts(self) -> (NodeId, PreparedScriptSourceLoadOutcome) {
        (self.node_id, self.outcome)
    }
}

impl ParseTimeDocumentScriptTask {
    pub(super) fn classic_async_script(
        script: PreparedScript,
        source_network_result: Option<SharedNavigationResponseResult>,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) -> Self {
        Self::ClassicAsyncScript(ParseTimeAsyncScript::new(
            script,
            source_network_result,
            load_delay_binding,
        ))
    }

    pub(super) fn async_script_failure(
        script: PreparedScript,
        error: String,
        source_network_result: Option<SharedNavigationResponseResult>,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) -> Self {
        Self::AsyncScriptFailure(ParseTimeAsyncScriptFailure {
            script: ParseTimeAsyncScript::new(script, None, load_delay_binding),
            error,
            source_network_result,
        })
    }
    #[cfg(test)]
    pub(crate) fn classic_async_script_for_test(script: PreparedScript) -> Self {
        Self::classic_async_script(script, None, None)
    }
}

impl ParseTimeAsyncScript {
    fn new(
        script: PreparedScript,
        source_network_result: Option<SharedNavigationResponseResult>,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) -> Self {
        Self {
            script,
            source_network_result,
            load_delay_binding,
        }
    }

    #[cfg(test)]
    pub(crate) fn script(&self) -> &PreparedScript {
        &self.script
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        PreparedScript,
        Option<SharedNavigationResponseResult>,
        Option<MainDocumentScriptLoadDelayLease>,
    ) {
        (
            self.script,
            self.source_network_result,
            self.load_delay_binding,
        )
    }
}

impl std::ops::Deref for ParseTimeAsyncScript {
    type Target = PreparedScript;

    fn deref(&self) -> &Self::Target {
        &self.script
    }
}

impl ParseTimeAsyncScriptFailure {
    #[cfg(test)]
    pub(crate) fn script(&self) -> &PreparedScript {
        self.script.script()
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        PreparedScript,
        String,
        Option<SharedNavigationResponseResult>,
        Option<MainDocumentScriptLoadDelayLease>,
    ) {
        let (script, _, load_delay_binding) = self.script.into_parts();
        (
            script,
            self.error,
            self.source_network_result,
            load_delay_binding,
        )
    }
}
