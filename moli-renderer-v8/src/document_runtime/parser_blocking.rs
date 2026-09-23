use super::*;
use crate::parser_script::main_document::PendingMainParserScript;

/// The Document owns one pending parser blocker, including synchronous input
/// inserted by document.write(). Executing work is taken out of this slot
/// before calling JavaScript: reentry may install its own blocker here.
#[derive(Debug)]
pub(super) enum PendingParserBlockingWork {
    MainScript(Box<PendingMainParserScript>),
    Insertion(Box<PendingParserInsertion>),
}

#[derive(Debug)]
pub(super) struct PendingParserInsertion {
    pub(super) insertion: SuspendedDocumentWriteInsertion,
    pub(super) blocking_signatures: HashSet<DocumentBlockingStylesheetSignature>,
    pub(super) work: ParserInsertionWork,
    pub(super) resume_after_completion: VecDeque<SuspendedDocumentWriteContinuation>,
}

#[derive(Debug)]
pub(super) enum ParserInsertionWork {
    StylesheetBoundary,
    Script {
        node: DomHandle,
        start_line: u64,
        start_column: u64,
        script: PreparedScript,
    },
    ExternalScript {
        target: crate::types::DocumentWriteExternalScriptFetchTarget,
        start: DocumentWriteExternalScriptStart,
        ready_completion: Option<Box<crate::types::DocumentWriteExternalScriptLoadCompletion>>,
    },
}

impl DocumentRuntime {
    pub(crate) fn pending_main_parser_script(&self) -> Option<&PendingMainParserScript> {
        match self.pending_parser_blocking_work.as_ref()? {
            PendingParserBlockingWork::MainScript(script) => Some(script),
            PendingParserBlockingWork::Insertion(_) => None,
        }
    }

    pub(crate) fn take_pending_main_parser_script(&mut self) -> Option<PendingMainParserScript> {
        self.pending_main_parser_script()?;
        let Some(PendingParserBlockingWork::MainScript(script)) =
            self.pending_parser_blocking_work.take()
        else {
            unreachable!()
        };
        Some(*script)
    }

    pub(crate) fn install_pending_main_parser_script(&mut self, script: PendingMainParserScript) {
        self.install_parser_blocking_work(PendingParserBlockingWork::MainScript(Box::new(script)));
    }

    fn install_parser_blocking_work(&mut self, work: PendingParserBlockingWork) {
        assert!(
            self.pending_parser_blocking_work.is_none(),
            "take the current parser blocker before installing another one"
        );
        self.pending_parser_blocking_work = Some(work);
    }

    pub(super) fn pending_parser_insertion(&self) -> Option<&PendingParserInsertion> {
        match self.pending_parser_blocking_work.as_ref()? {
            PendingParserBlockingWork::Insertion(pending) => Some(pending),
            PendingParserBlockingWork::MainScript(_) => None,
        }
    }

    pub(super) fn pending_parser_insertion_mut(&mut self) -> Option<&mut PendingParserInsertion> {
        match self.pending_parser_blocking_work.as_mut()? {
            PendingParserBlockingWork::Insertion(pending) => Some(pending),
            PendingParserBlockingWork::MainScript(_) => None,
        }
    }

    pub(super) fn take_pending_parser_insertion(&mut self) -> Option<PendingParserInsertion> {
        self.pending_parser_insertion()?;
        let Some(PendingParserBlockingWork::Insertion(pending)) =
            self.pending_parser_blocking_work.take()
        else {
            unreachable!()
        };
        Some(*pending)
    }

    pub(super) fn install_pending_parser_insertion(&mut self, pending: PendingParserInsertion) {
        self.install_parser_blocking_work(PendingParserBlockingWork::Insertion(Box::new(pending)));
    }
}
