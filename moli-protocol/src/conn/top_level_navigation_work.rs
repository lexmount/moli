use moli_core::page::RendererDocumentSourcedTopLevelLocationNavigation;

use super::{CommandOwnerScope, TargetPageResidenceIdentity};

/// One renderer-requested top-level navigation projected from a concrete
/// renderer output record.
///
/// The renderer freezes and moves the request at its producing turn boundary.
/// Protocol only projects that immutable record into scheduler work; it never
/// rescans mutable Page state. The action retains both identities needed at its
/// eventual turn:
///
/// - `page_owner` rejects execution after Page replacement or retirement;
/// - `owner_scope` restores the target route used by an implicit (`None`)
///   session instead of resolving whichever target is current later.
///
/// The source Document remains inside `navigation` as causal metadata. A
/// same-Page `document.open()` replacement therefore does not manufacture a
/// new request, while replacement of the Page residence makes the action
/// stale.
#[derive(Debug)]
pub(crate) struct TopLevelLocationNavigationOwnerAction {
    owner_scope: CommandOwnerScope,
    page_owner: TargetPageResidenceIdentity,
    navigation: RendererDocumentSourcedTopLevelLocationNavigation,
}

impl TopLevelLocationNavigationOwnerAction {
    /// Wraps a navigation that has already been claimed while its exact
    /// protocol route is installed.
    ///
    /// Preparing renderer output moves the navigation out of the Page before
    /// the output slot is drained. Draining must therefore publish this
    /// concrete action rather than execute the navigation inline or rescan the
    /// current Page.
    pub(crate) fn from_prepared(
        owner_scope: CommandOwnerScope,
        page_owner: TargetPageResidenceIdentity,
        navigation: RendererDocumentSourcedTopLevelLocationNavigation,
    ) -> Self {
        Self {
            owner_scope,
            page_owner,
            navigation,
        }
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }

    pub(crate) fn target_id(&self) -> Option<&str> {
        self.page_owner.target_id()
    }

    pub(crate) fn source_document(&self) -> moli_core::RendererDocumentLifecycleIdentity {
        self.navigation.source_document()
    }

    pub(crate) fn url(&self) -> &str {
        self.navigation.url()
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        CommandOwnerScope,
        TargetPageResidenceIdentity,
        RendererDocumentSourcedTopLevelLocationNavigation,
    ) {
        (self.owner_scope, self.page_owner, self.navigation)
    }
}
