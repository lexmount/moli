#[cfg(feature = "devtools")]
use moli_layout::LayoutTransform2D;
use moli_layout::{FrozenLayoutTree, LayoutError, LayoutPoint, ScrollProjectionRefresh};
#[cfg(feature = "devtools")]
use std::{collections::HashMap, rc::Rc};

use crate::document_runtime::DomHandle;

#[cfg(feature = "devtools")]
use super::{PageScriptHost, WindowDocumentTaskTarget};

/// Viewport transforms from the same publication as the frozen tree. The
/// Document handle is an index into the sample. The exact Window/Document
/// owner must still match before its transform can be used.
#[cfg(feature = "devtools")]
pub(crate) struct PublishedFrameGeometry {
    root_owner: Option<WindowDocumentTaskTarget>,
    frames: HashMap<DomHandle, (WindowDocumentTaskTarget, LayoutTransform2D)>,
}

#[cfg(feature = "devtools")]
impl PublishedFrameGeometry {
    pub(super) fn new(root_owner: Option<WindowDocumentTaskTarget>) -> Self {
        Self {
            root_owner,
            frames: HashMap::new(),
        }
    }

    pub(super) fn insert(
        &mut self,
        document: DomHandle,
        owner: WindowDocumentTaskTarget,
        root_to_viewport: LayoutTransform2D,
    ) {
        self.frames.insert(document, (owner, root_to_viewport));
    }

    pub(crate) fn root_to_viewport(
        &self,
        host: &PageScriptHost,
        document: DomHandle,
    ) -> Option<LayoutTransform2D> {
        let root_owner = self.root_owner?;
        if host.input_document_for_owner(root_owner) != Some(host.document_handle()) {
            return None;
        }
        let (owner, transform) = *self.frames.get(&document)?;
        if host.input_document_for_owner(owner) != Some(document) {
            return None;
        }
        Some(transform)
    }
}

struct LatestFrozenLayout {
    document: DomHandle,
    tree: Box<FrozenLayoutTree<DomHandle>>,
    #[cfg(feature = "devtools")]
    frame_geometry: Rc<PublishedFrameGeometry>,
}

/// Single-slot storage for the latest successful frozen layout tree.
///
/// This owns exactly one recursively frozen snapshot. It has no separately
/// keyed child tree, working layout world, source index, hit-test index, Taffy
/// cache, style borrow, pass diagnostics, paint snapshot, timer, or invalidation
/// policy. The first geometry demand initializes it. Screenshots, screencast
/// frames and explicit layout refreshes replace
/// this published layout; print projections never enter this cache.
#[derive(Default)]
pub(super) struct LatestLayoutTreeCache {
    latest: Option<LatestFrozenLayout>,
}

impl LatestLayoutTreeCache {
    pub(super) fn get(&self, document: DomHandle) -> Option<&FrozenLayoutTree<DomHandle>> {
        self.latest
            .as_ref()
            .filter(|snapshot| snapshot.document == document)
            .map(|snapshot| snapshot.tree.as_ref())
    }

    pub(super) fn get_for_root(&self, root: DomHandle) -> Option<&FrozenLayoutTree<DomHandle>> {
        self.latest.as_ref()?.tree.tree_for_root(root)
    }

    pub(super) fn latest_tree_mut(&mut self) -> Option<&mut FrozenLayoutTree<DomHandle>> {
        self.latest.as_mut().map(|snapshot| snapshot.tree.as_mut())
    }

    #[cfg(feature = "devtools")]
    pub(super) fn frame_geometry(&self) -> Option<&Rc<PublishedFrameGeometry>> {
        self.latest
            .as_ref()
            .map(|snapshot| &snapshot.frame_geometry)
    }

    pub(super) fn publish(
        &mut self,
        document: DomHandle,
        tree: FrozenLayoutTree<DomHandle>,
        #[cfg(feature = "devtools")] frame_geometry: PublishedFrameGeometry,
    ) {
        self.latest = Some(LatestFrozenLayout {
            document,
            tree: Box::new(tree),
            #[cfg(feature = "devtools")]
            frame_geometry: Rc::new(frame_geometry),
        });
    }

    pub(super) fn replace_projection(&mut self, tree: FrozenLayoutTree<DomHandle>) {
        let snapshot = self
            .latest
            .as_mut()
            .expect("projection needs published layout");
        *snapshot.tree = tree;
    }

    pub(super) fn refresh_scroll_projection(
        &mut self,
        document: DomHandle,
        offset: impl Fn(DomHandle) -> LayoutPoint,
        #[cfg(feature = "devtools")] frame_geometry: impl FnOnce(
            &FrozenLayoutTree<DomHandle>,
        ) -> PublishedFrameGeometry,
    ) -> Result<ScrollProjectionRefresh, LayoutError> {
        let Some(snapshot) = self
            .latest
            .as_mut()
            .filter(|snapshot| snapshot.document == document)
        else {
            return Ok(ScrollProjectionRefresh::Unavailable);
        };
        let outcome = snapshot.tree.refresh_scroll_projection(offset)?;
        #[cfg(feature = "devtools")]
        if outcome == ScrollProjectionRefresh::Changed {
            snapshot.frame_geometry = Rc::new(frame_geometry(&snapshot.tree));
        }
        Ok(outcome)
    }

    pub(super) fn clear(&mut self) {
        self.latest = None;
    }

    #[cfg(test)]
    pub(super) fn observability(
        &self,
    ) -> Option<(DomHandle, moli_layout::LayoutTreeRetentionMetrics)> {
        self.latest
            .as_ref()
            .map(|snapshot| (snapshot.document, snapshot.tree.retention_metrics()))
    }
}
