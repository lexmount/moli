#[cfg(test)]
use crate::live_document_parser::DocumentParserLifetime;
use crate::{
    frame_owner_model::FrameDocumentOwner,
    live_document_parser::{
        DocumentParserSession, ParserResumePermit, ParserStopReason, ParserSuspensionCause,
    },
};
use std::collections::HashMap;

#[derive(Default)]
pub(in crate::native_bridge::context_host) struct ChildDocumentParserStore {
    sessions: HashMap<FrameDocumentOwner, DocumentParserSession>,
}

impl ChildDocumentParserStore {
    pub(in crate::native_bridge::context_host) fn clear(&mut self, owner: FrameDocumentOwner) {
        if let Some(mut parser) = self.sessions.remove(&owner) {
            parser.stop(ParserStopReason::DocumentReplacement);
        }
    }

    pub(in crate::native_bridge::context_host) fn replace(
        &mut self,
        owner: FrameDocumentOwner,
        parser: DocumentParserSession,
    ) {
        if let Some(mut replaced) = self.sessions.insert(owner, parser) {
            replaced.stop(ParserStopReason::DocumentReplacement);
        }
    }

    pub(in crate::native_bridge::context_host) fn take(
        &mut self,
        owner: FrameDocumentOwner,
    ) -> Option<DocumentParserSession> {
        self.sessions.remove(&owner)
    }

    #[cfg(test)]
    pub(in crate::native_bridge::context_host) fn has_open_stream(
        &self,
        owner: FrameDocumentOwner,
    ) -> bool {
        self.sessions.get(&owner).is_some_and(|entry| {
            matches!(
                entry.lifetime(),
                DocumentParserLifetime::Open | DocumentParserLifetime::Closing
            )
        })
    }

    pub(in crate::native_bridge::context_host) fn contains(
        &self,
        owner: FrameDocumentOwner,
    ) -> bool {
        self.sessions.contains_key(&owner)
    }

    pub(in crate::native_bridge::context_host) fn parser_script_resume_permit(
        &self,
        owner: FrameDocumentOwner,
        script: crate::document_runtime::DomHandle,
    ) -> Option<ParserResumePermit> {
        let parser = self.sessions.get(&owner)?;
        if !matches!(
            parser.suspension_cause(),
            Some(
                ParserSuspensionCause::ParserClassicSource {
                    script: suspended_script,
                } | ParserSuspensionCause::ParserClassicStylesheets {
                    script: suspended_script,
                }
            ) if suspended_script == script
        ) {
            return None;
        }
        parser.current_resume_permit()
    }

    pub(in crate::native_bridge::context_host) fn resume_parser_script_for_execution(
        &mut self,
        owner: FrameDocumentOwner,
        permit: ParserResumePermit,
    ) -> Option<bool> {
        self.sessions
            .get_mut(&owner)
            .map(|parser| parser.resume(permit))
    }

    pub(in crate::native_bridge::context_host) fn is_suspended_on_parser_created_stylesheet(
        &self,
        owner: FrameDocumentOwner,
    ) -> bool {
        self.sessions
            .get(&owner)
            .and_then(DocumentParserSession::suspension_cause)
            .is_some_and(|cause| {
                matches!(cause, ParserSuspensionCause::ParserCreatedStylesheet { .. })
            })
    }

    #[cfg(test)]
    pub(in crate::native_bridge::context_host) fn is_complete_for(
        &self,
        owner: FrameDocumentOwner,
    ) -> bool {
        !self.sessions.contains_key(&owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        dom::native::NativeNodeId,
        frame_owner_model::{DocumentId, LocalWindowId},
    };
    use url::Url;

    fn test_parser() -> DocumentParserSession {
        DocumentParserSession::start_finite_live_document(
            Url::parse("https://child-parser-store.test/").expect("test url"),
            NativeNodeId::new(1),
        )
    }

    #[test]
    fn child_document_parser_store_replaces_and_takes_by_owner() {
        let owner = FrameDocumentOwner::new(LocalWindowId(1), DocumentId(2));
        let other = FrameDocumentOwner::new(LocalWindowId(1), DocumentId(3));
        let mut store = ChildDocumentParserStore::default();

        assert!(store.is_complete_for(owner));

        store.replace(owner, test_parser());
        assert!(!store.is_complete_for(owner));
        assert!(store.is_complete_for(other));
        assert_eq!(
            store
                .take(owner)
                .expect("finite child parser session")
                .lifetime(),
            DocumentParserLifetime::Finite
        );

        store.replace(
            owner,
            DocumentParserSession::start_open_live_document(
                Url::parse("https://child-parser-store.test/").expect("test url"),
                NativeNodeId::new(1),
            ),
        );
        assert!(!store.is_complete_for(owner));
        assert!(store.has_open_stream(owner));

        assert!(store.take(other).is_none());
        let mut entry = store.take(owner).expect("open parser entry");
        assert_eq!(entry.lifetime(), DocumentParserLifetime::Open);
        entry.request_close();
        assert!(entry.finishes_on_empty_input());
        assert!(store.is_complete_for(owner));
    }
}
