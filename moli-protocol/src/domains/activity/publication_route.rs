use moli_core::RendererOutputResidenceIdentity;

use crate::conn::{CdpConnection, CdpSessionRoute, RendererPageResidenceIdentity};

/// Protocol owner frozen when one renderer stream opens.
///
/// A Page can publish its final batch before the asynchronous transport is
/// drained, while the protocol owner has already installed its replacement
/// Page. Resolving every batch through the *current* Page would therefore lose
/// the old stream's target. The target identity is stable across that window;
/// the concrete session remains deliberately dynamic because sessions may
/// attach or detach without restarting the renderer stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RendererPublicationOwner {
    PageTarget {
        browser_context_id: String,
        target_id: Option<String>,
        renderer_page: RendererPageResidenceIdentity,
        page_owner: crate::conn::TargetPageResidenceIdentity,
    },
    BrowserContext {
        browser_context_id: String,
    },
}

/// Exact protocol delivery route selected for one renderer publication.
///
/// An attached session is already an exact route. A publication without a
/// session instead carries the owner route needed to enter the correct parked
/// target without promoting it into the active target slot. This type contains
/// no output payload and grants no renderer execution authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RendererPublicationRoute {
    AttachedSession {
        session_id: String,
        projection: RendererPublicationProjection,
    },
    UnattachedOwner {
        owner_route: CdpSessionRoute,
        projection: RendererPublicationProjection,
    },
}

/// A current Page can project its complete renderer stream. A replaced Page
/// remains routable only for final Network facts whose request correlations
/// are retained by the target; every other historical record is stale.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPublicationProjection {
    CurrentOwner,
    RetiringNetworkOnly,
}

impl RendererPublicationRoute {
    fn for_target(
        browser_context_id: String,
        target_id: String,
        session_id: Option<String>,
        projection: RendererPublicationProjection,
    ) -> Self {
        if let Some(session_id) = session_id {
            return Self::AttachedSession {
                session_id,
                projection,
            };
        }
        let owner_route = CdpSessionRoute::PageTarget {
            browser_context_id,
            target_id,
            session_key: moli_page_types::DevToolsSessionKey::Primary,
        };
        Self::UnattachedOwner {
            owner_route,
            projection,
        }
    }
}

pub(crate) fn renderer_publication_owners(
    conn: &CdpConnection,
    residence: RendererOutputResidenceIdentity,
) -> Vec<RendererPublicationOwner> {
    match residence {
        // A Page stream is bound by the navigation/initial-document
        // transaction that reserved that exact renderer Page. Inferring its
        // target from the mutable inventory at `Opened` time is ambiguous:
        // protocol can transiently retain two handles to the same Page while
        // moving a target between active/background residence. Leave Page
        // discovery empty and let the explicit binding win in either
        // open-before-bind or bind-before-open order.
        RendererOutputResidenceIdentity::Page { .. } => Vec::new(),
        RendererOutputResidenceIdentity::SharedWorker {
            browser_context_runtime_id,
            ..
        }
        | RendererOutputResidenceIdentity::ServiceWorker {
            browser_context_runtime_id,
            ..
        } => conn
            .browser_context
            .iter()
            .chain(conn.inactive_browser_contexts.iter())
            .filter(|browser_context| {
                browser_context.routes_renderer_browser_context_runtime(browser_context_runtime_id)
            })
            .map(|browser_context| RendererPublicationOwner::BrowserContext {
                browser_context_id: browser_context.id.clone(),
            })
            .collect(),
    }
}

impl RendererPublicationOwner {
    /// Resolves the current session projection for a stable renderer owner.
    ///
    /// Returning `None` means the target/browser context was retired after
    /// this stream opened. Its already-admitted cursor remains settled, but no
    /// historical output may be projected into a replacement owner.
    pub(crate) fn resolve(&self, conn: &CdpConnection) -> Option<RendererPublicationRoute> {
        match self {
            Self::BrowserContext { browser_context_id } => {
                if !conn
                    .browser_context
                    .iter()
                    .chain(conn.inactive_browser_contexts.iter())
                    .any(|browser_context| browser_context.id == *browser_context_id)
                {
                    return None;
                }
                Some(RendererPublicationRoute::UnattachedOwner {
                    owner_route: CdpSessionRoute::BrowserContext {
                        browser_context_id: browser_context_id.clone(),
                    },
                    projection: RendererPublicationProjection::CurrentOwner,
                })
            }
            Self::PageTarget {
                browser_context_id,
                target_id,
                renderer_page,
                page_owner,
                ..
            } => conn
                .browser_context
                .iter()
                .chain(conn.inactive_browser_contexts.iter())
                .filter(|browser_context| browser_context.id == *browser_context_id)
                .find_map(|browser_context| {
                    let projection_for = |runtime_slot: &crate::conn::TargetRuntimeSlot| {
                        if runtime_slot.routes_current_renderer_page_owner(
                            *renderer_page,
                            page_owner.page_attachment_id(),
                        ) {
                            Some(RendererPublicationProjection::CurrentOwner)
                        } else if runtime_slot.routes_retiring_renderer_page_owner(
                            *renderer_page,
                            page_owner.page_attachment_id(),
                        ) {
                            Some(RendererPublicationProjection::RetiringNetworkOnly)
                        } else {
                            None
                        }
                    };
                    let route_for =
                        |runtime_slot: &crate::conn::TargetRuntimeSlot,
                         route_target_id: String,
                         session_id: Option<String>| {
                            projection_for(runtime_slot).map(|projection| {
                                RendererPublicationRoute::for_target(
                                    browser_context.id.clone(),
                                    route_target_id,
                                    session_id,
                                    projection,
                                )
                            })
                        };

                    // Prefer the target route captured when the renderer stream
                    // was bound. This disambiguates the brief handoff window in
                    // which active and parked state can both refer to one Page.
                    let frozen_target = target_id
                        .as_deref()
                        .and_then(|target_id| browser_context.page_target(target_id))
                        .or_else(|| {
                            target_id
                                .is_none()
                                .then(|| browser_context.page_targets.active())
                                .flatten()
                        });
                    let frozen_route = frozen_target.and_then(|target| {
                        route_for(
                            target.runtime_slot(),
                            target.target_id().to_owned(),
                            target.session_id().map(str::to_owned),
                        )
                    });
                    if frozen_route.is_some() {
                        return frozen_route;
                    }

                    // A target can acquire its externally visible id after its
                    // initial Page was installed. The Page residence and Page
                    // attachment id remain stable across that rename, so use
                    // those identities to recover the current concrete route.
                    browser_context.page_targets.iter().find_map(|target| {
                        route_for(
                            target.runtime_slot(),
                            target.target_id().to_owned(),
                            target.session_id().map(str::to_owned),
                        )
                    })
                }),
        }
    }
}
