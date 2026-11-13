use moli_core::page::{
    ChildFrameDocumentNetworkActivitySnapshot, ChildFrameDocumentNetworkSnapshot,
    ChildFrameDocumentOpenedSnapshot, ChildFrameNavigationSnapshot,
};

use crate::conn::TargetRootDocumentProtocolAttachmentIdentity;

use super::LOADER_ID;

/// One prepared child-frame activity batch and the exact protocol/root
/// Document authority under which it may perform browser-owner actions.
///
/// The renderer V8 Inspector stream owns Runtime context lifecycle output.
/// Child-frame activity therefore contains only DOM/frame/network facts and
/// cannot rescan live realms or absorb a second Runtime half.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PagePreparedChildFrameActivity {
    binding: TargetRootDocumentProtocolAttachmentIdentity,
    document: PagePreparedChildFrameDocumentActivity,
}

impl PagePreparedChildFrameActivity {
    pub(crate) fn from_document(
        binding: TargetRootDocumentProtocolAttachmentIdentity,
        document: PagePreparedChildFrameDocumentActivity,
    ) -> Self {
        Self { binding, document }
    }

    pub(crate) fn binding(&self) -> &TargetRootDocumentProtocolAttachmentIdentity {
        &self.binding
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        TargetRootDocumentProtocolAttachmentIdentity,
        PagePreparedChildFrameDocumentActivity,
    ) {
        (self.binding, self.document)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PagePreparedChildFrameDocumentActivity {
    pub(super) timestamp: f64,
    pub(super) child_frame_tree_events: Vec<PagePreparedChildFrameTreeEvent>,
    pub(super) document_opened_events: Vec<ChildFrameDocumentOpenedSnapshot>,
    pub(super) document_networks: Vec<PagePreparedChildFrameDocumentNetwork>,
    pub(super) commits: Vec<PagePreparedChildFrameNavigationCommit>,
    pub(super) loads: Vec<PagePreparedChildFrameLoadActivity>,
    pub(super) security_origin: String,
    pub(super) secure_context_type: String,
}

impl PagePreparedChildFrameDocumentActivity {
    pub(super) fn from_parts(
        timestamp: f64,
        child_frame_tree_events: Vec<PagePreparedChildFrameTreeEvent>,
        document_opened_events: Vec<ChildFrameDocumentOpenedSnapshot>,
        document_networks: Vec<ChildFrameDocumentNetworkActivitySnapshot>,
        loads: Vec<ChildFrameNavigationSnapshot>,
        security_origin: String,
        secure_context_type: String,
    ) -> Self {
        Self {
            timestamp,
            child_frame_tree_events,
            document_opened_events,
            commits: Vec::new(),
            document_networks: document_networks
                .into_iter()
                .map(|network| PagePreparedChildFrameDocumentNetwork {
                    frame_id: network.frame_id,
                    loader_id: network.loader_id,
                    timestamp,
                    snapshot: network.snapshot,
                })
                .collect(),
            loads: loads
                .into_iter()
                .map(|load| PagePreparedChildFrameLoadActivity::from_snapshot(load, timestamp))
                .collect(),
            security_origin,
            secure_context_type,
        }
    }
}

/// Ordered child-frame tree mutation captured under the enclosing exact root
/// Document attachment.
///
/// Attach and detach must remain in one stream: collapsing this to an
/// attachment snapshot would lose a detach that races with output capture and
/// could leave protocol target state pointing at a retired child frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PagePreparedChildFrameTreeEvent {
    Attached {
        frame_id: String,
        parent_frame_id: String,
    },
    Detached {
        frame_id: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PagePreparedChildFrameLoadActivity {
    pub(super) document_open_replacement: bool,
    pub(super) navigation_commit: PagePreparedChildFrameNavigationCommit,
    pub(super) lifecycle_terminal: PagePreparedChildFrameLifecycleTerminal,
}

impl PagePreparedChildFrameLoadActivity {
    fn from_snapshot(load: ChildFrameNavigationSnapshot, timestamp: f64) -> Self {
        let document_open_replacement = load.document_open_replacement;
        let navigation_commit = PagePreparedChildFrameNavigationCommit::from_snapshot(load);
        let lifecycle_terminal = PagePreparedChildFrameLifecycleTerminal {
            frame_id: navigation_commit.frame_id.clone(),
            loader_id: navigation_commit.loader_id.clone(),
            timestamp,
        };
        Self {
            document_open_replacement,
            navigation_commit,
            lifecycle_terminal,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PagePreparedChildFrameDocumentNetwork {
    pub(super) frame_id: String,
    pub(super) loader_id: String,
    pub(super) timestamp: f64,
    pub(super) snapshot: ChildFrameDocumentNetworkSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PagePreparedChildFrameNavigationCommit {
    pub(super) frame_id: String,
    pub(super) parent_frame_id: Option<String>,
    pub(super) name: Option<String>,
    pub(super) loader_id: String,
    /// The renderer-assigned child Document loader that owns runtime-world
    /// creation. `None` is retained only for legacy/synthetic event shapes and
    /// never authorizes a current-child owner action.
    pub(super) exact_loader_id: Option<String>,
    pub(super) url: String,
    pub(super) security_origin_inherited: bool,
    pub(super) security_origin_opaque: bool,
}

impl PagePreparedChildFrameNavigationCommit {
    pub(super) fn from_snapshot(snapshot: ChildFrameNavigationSnapshot) -> Self {
        Self {
            frame_id: snapshot.frame_id,
            parent_frame_id: snapshot.parent_frame_id,
            name: snapshot.name,
            loader_id: snapshot
                .loader_id
                .clone()
                .unwrap_or_else(|| LOADER_ID.to_owned()),
            exact_loader_id: snapshot.loader_id,
            url: snapshot.url,
            security_origin_inherited: snapshot.security_origin_inherited,
            security_origin_opaque: snapshot.security_origin_opaque,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PagePreparedChildFrameLifecycleTerminal {
    pub(super) frame_id: String,
    pub(super) loader_id: String,
    pub(super) timestamp: f64,
}
