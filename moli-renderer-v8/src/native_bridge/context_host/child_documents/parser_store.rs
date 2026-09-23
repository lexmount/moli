#[cfg(test)]
use crate::live_document_parser::DocumentParserLifetime;
use crate::{
    frame_owner_model::FrameDocumentOwner,
    live_document_parser::{
        DocumentParserRunState, DocumentParserSession, DocumentParserSessionControlHandle,
        ParserInsertionHandle, ParserResumeOwner, ParserResumePermit, ParserStopReason,
        ParserSuspensionCause,
    },
};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

#[derive(Default)]
pub(in crate::native_bridge::context_host) struct ChildDocumentParserStore {
    sessions: HashMap<FrameDocumentOwner, DocumentParserSession>,
    // A finite parser is owned by its driver while running. Expose only its
    // insertion capability for synchronous callbacks, until that step unwinds.
    active: Rc<RefCell<Vec<(FrameDocumentOwner, ParserInsertionHandle)>>>,
}

pub(in crate::native_bridge::context_host) struct ActiveChildParser {
    active: Rc<RefCell<Vec<(FrameDocumentOwner, ParserInsertionHandle)>>>,
    owner: FrameDocumentOwner,
}

impl Drop for ActiveChildParser {
    fn drop(&mut self) {
        let (owner, _) = self.active.borrow_mut().pop().expect("active parser scope");
        assert_eq!(
            owner, self.owner,
            "active parser scopes must unwind in order"
        );
    }
}

impl ChildDocumentParserStore {
    pub(in crate::native_bridge::context_host) fn clear(&mut self, owner: FrameDocumentOwner) {
        for (active_owner, handle) in self.active.borrow().iter() {
            if *active_owner == owner {
                handle.stop(ParserStopReason::DocumentReplacement);
            }
        }
        if let Some(mut parser) = self.sessions.remove(&owner) {
            parser.stop(ParserStopReason::DocumentReplacement);
        }
    }

    pub(in crate::native_bridge::context_host) fn enter_active(
        &self,
        owner: FrameDocumentOwner,
        handle: ParserInsertionHandle,
    ) -> ActiveChildParser {
        self.active.borrow_mut().push((owner, handle));
        ActiveChildParser {
            active: self.active.clone(),
            owner,
        }
    }

    pub(in crate::native_bridge::context_host) fn insertion_handle(
        &self,
        owner: FrameDocumentOwner,
    ) -> Option<ParserInsertionHandle> {
        self.sessions
            .get(&owner)
            .and_then(DocumentParserSession::insertion_handle)
            .or_else(|| {
                self.active
                    .borrow()
                    .iter()
                    .rev()
                    .find_map(|(active_owner, handle)| {
                        (*active_owner == owner
                            && !matches!(handle.run_state(), DocumentParserRunState::Stopped(_)))
                        .then(|| handle.clone())
                    })
            })
    }

    fn control_handle(
        &self,
        owner: FrameDocumentOwner,
    ) -> Option<DocumentParserSessionControlHandle> {
        self.sessions
            .get(&owner)
            .map(DocumentParserSession::control_handle)
            .or_else(|| {
                self.insertion_handle(owner)
                    .map(|handle| handle.control_handle())
            })
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
        self.sessions.contains_key(&owner) || self.insertion_handle(owner).is_some()
    }

    pub(in crate::native_bridge::context_host) fn parser_script_resume_permit(
        &self,
        owner: FrameDocumentOwner,
        script: crate::document_runtime::DomHandle,
    ) -> Option<ParserResumePermit> {
        let parser = self.control_handle(owner)?;
        if !matches!(
            parser.run_state(),
            DocumentParserRunState::Suspended { cause:
                ParserSuspensionCause::ParserClassicSource {
                    script: suspended_script,
                } | ParserSuspensionCause::ParserClassicStylesheets {
                    script: suspended_script,
                }
            , .. } if suspended_script == script
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
        self.control_handle(owner)
            .map(|parser| parser.resume(permit, ParserResumeOwner::ParserDriver))
    }

    pub(in crate::native_bridge::context_host) fn is_suspended_on_parser_created_stylesheet(
        &self,
        owner: FrameDocumentOwner,
    ) -> bool {
        self.control_handle(owner).is_some_and(|control| {
            matches!(
                control.run_state(),
                DocumentParserRunState::Suspended {
                    cause: ParserSuspensionCause::ParserCreatedStylesheet { .. },
                    ..
                }
            )
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
            true,
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
                true,
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

    #[test]
    fn active_child_parser_retains_writes_and_close_while_suspended() {
        let owner = FrameDocumentOwner::new(LocalWindowId(1), DocumentId(2));
        let mut parser = DocumentParserSession::start_open_live_document(
            Url::parse("https://child-parser-store.test/").unwrap(),
            NativeNodeId::new(1),
            true,
        );
        let store = ChildDocumentParserStore::default();
        let active = store.enter_active(owner, parser.insertion_handle().unwrap());
        let permit = parser.suspend(ParserSuspensionCause::ParserClassicSource {
            script: NativeNodeId::new(2),
        });
        let insertion = store.insertion_handle(owner).unwrap();
        insertion.queue_arrived_chunk("<p>first</p>".to_owned());
        insertion.queue_arrived_chunk("<p>second</p>".to_owned());
        assert_eq!(
            insertion.request_close(),
            crate::live_document_parser::DocumentParserCloseDisposition::DeferredUntilReady
        );
        drop(insertion);
        drop(active);

        assert_eq!(parser.snapshot_pending_input(), "<p>first</p><p>second</p>");
        assert_eq!(parser.lifetime(), DocumentParserLifetime::Closing);
        assert!(parser.resume(permit));
        assert_eq!(parser.run_state(), DocumentParserRunState::Ready);
    }

    #[test]
    fn replacing_document_cancels_active_child_parser_access() {
        let owner = FrameDocumentOwner::new(LocalWindowId(1), DocumentId(2));
        let parser = test_parser();
        let mut store = ChildDocumentParserStore::default();
        let _active = store.enter_active(owner, parser.insertion_handle().unwrap());
        store.clear(owner);
        assert!(!store.contains(owner));
        assert_eq!(
            parser.run_state(),
            DocumentParserRunState::Stopped(ParserStopReason::DocumentReplacement)
        );
    }
}
