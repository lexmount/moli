use super::parser_blocking_owner::MainParserBlockingExternalLoadOwner;
use crate::DocumentBlockingStylesheetSignature;
use crate::frame_owner_model::FrameDocumentTaskOwner;
use crate::parser_script::item::ParserClassicScriptRunnerItem;
pub(super) use crate::parser_script::main_document::{
    PendingParserBlockingSourceLoad, PendingParsingBlockingClassicScriptContext,
};
#[cfg(test)]
use crate::parser_script::payload::ParserClassicScriptMetadata;
use crate::parser_script::payload::ParserPreparedClassicScript;
use crate::parser_script::runner::ParserClassicScriptRunner;
#[cfg(test)]
use crate::planning::PreparedScript;
use std::collections::HashSet;

pub(super) type PendingParsingBlockingClassicScript =
    ParserClassicScriptRunnerItem<PendingParsingBlockingClassicScriptContext>;
pub(super) type PendingParsingBlockingClassicScriptRunner =
    ParserClassicScriptRunner<PendingParsingBlockingClassicScriptContext>;

pub(super) fn main_parser_blocking_classic_script_item(
    owner: FrameDocumentTaskOwner,
    input: ParserPreparedClassicScript,
    blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
    source_load: Option<PendingParserBlockingSourceLoad>,
) -> PendingParsingBlockingClassicScript {
    let has_source_load = source_load.is_some();
    let is_ready = input.ready_script().is_some();
    let context =
        PendingParsingBlockingClassicScriptContext::new(owner, blocking_signatures_before, None);
    let mut item = if has_source_load || !is_ready {
        ParserClassicScriptRunnerItem::external_pending(input, context)
    } else {
        ParserClassicScriptRunnerItem::inline_ready(input, context)
    };
    if let Some(source_load) = source_load {
        let mut owner = MainParserBlockingExternalLoadOwner::new(owner, source_load);
        let _ = item
            .begin_runner_external_load_with_load_id_and_owner(None, &mut owner)
            .expect("parser-blocking source load must move pending script into loading state");
        debug_assert!(
            owner.source_load_transferred(),
            "parser-blocking source load owner must transfer load into context"
        );
    }
    item
}

#[cfg(test)]
pub(super) fn parser_blocking_classic_script_for_test(
    script: &PendingParsingBlockingClassicScript,
) -> Option<&PreparedScript> {
    script.runner_script()
}

#[cfg(test)]
pub(super) fn parser_blocking_classic_metadata_for_test(
    script: &PendingParsingBlockingClassicScript,
) -> Option<ParserClassicScriptMetadata> {
    script.runner_metadata()
}

#[cfg(test)]
pub(super) fn parser_blocking_classic_source_load_for_test(
    script: &PendingParsingBlockingClassicScript,
) -> Option<&PendingParserBlockingSourceLoad> {
    script.context().source_load.as_ref()
}
