use moli_core::page::{
    RendererPendingPopupActivation, RendererPendingWindowOpenEvent, RendererPopupActivationSource,
    RendererWindowDocumentSource,
};

use crate::{
    conn::{BackgroundProtocolEvent, CdpConnection, TargetPageResidenceIdentity},
    domains::target::{PopupTargetCreation, PopupTargetOpenerIdentity},
};

/// One renderer-accepted auxiliary browsing-context action after it has left
/// the renderer Page.
///
/// `page_owner` freezes the browser context and top-level target that carried
/// the renderer Page. It is provenance, not a late currentness gate: Chromium
/// accepts `window.open()` synchronously, so closing or replacing the source
/// Document after that point must not cancel creation of the auxiliary
/// browsing context. The exact renderer Window source remains attached so
/// opener projection never falls back to the session or target current at
/// emission time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PagePreparedPopupActivation {
    page_owner: TargetPageResidenceIdentity,
    activation: RendererPendingPopupActivation,
}

impl PagePreparedPopupActivation {
    pub(super) fn new(
        page_owner: TargetPageResidenceIdentity,
        activation: RendererPendingPopupActivation,
    ) -> Self {
        Self {
            page_owner,
            activation,
        }
    }
}

/// One already-accepted `window.open()` observation and the Page sessions
/// subscribed when its concrete renderer record crossed the protocol boundary.
///
/// This is deliberately separate from [`PagePreparedPopupActivation`]:
/// `Page.windowOpen` is a protocol observation, while auxiliary browsing-
/// context creation is a listener-independent browser-owner action. Chromium
/// places both the observation and `Target.targetCreated` before the causing
/// Runtime response; keeping separate typed records preserves that ordering
/// without making target creation depend on Page-domain subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PagePreparedWindowOpenEvent {
    session_ids: Vec<Option<String>>,
    event: RendererPendingWindowOpenEvent,
}

impl PagePreparedWindowOpenEvent {
    pub(super) fn new(
        session_ids: Vec<Option<String>>,
        event: RendererPendingWindowOpenEvent,
    ) -> Self {
        Self { session_ids, event }
    }
}

pub(super) fn emit_window_open_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    events: Vec<PagePreparedWindowOpenEvent>,
) {
    for prepared in events {
        for session_id in prepared.session_ids {
            out.push(BackgroundProtocolEvent::page_window_open(
                session_id.as_deref(),
                &prepared.event.url,
                &prepared.event.window_name,
                &prepared.event.window_features,
                prepared.event.user_gesture,
            ));
        }
    }
}

pub(super) async fn emit_prepared(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    activations: Vec<PagePreparedPopupActivation>,
) {
    for prepared in activations {
        let PagePreparedPopupActivation {
            page_owner,
            activation,
        } = prepared;
        let (
            source,
            popup_id,
            url,
            target_name,
            session_storage_store,
            initial_empty_document_storage_key,
        ) = activation.into_parts();
        let can_access_opener = matches!(
            &source,
            RendererPopupActivationSource::Window {
                exposes_opener: true,
                ..
            }
        );
        let opener = resolve_devtools_opener(conn, &page_owner, &source);
        let creation = PopupTargetCreation::new(
            page_owner.browser_context_id().to_owned(),
            popup_id,
            url,
            target_name,
            opener,
            can_access_opener,
            session_storage_store,
            initial_empty_document_storage_key,
        );
        let browser_context_id = page_owner.browser_context_id().to_owned();
        let target_id =
            crate::domains::target::create_popup_target_from_renderer_output_background_events_async(
                conn, out, creation,
            )
            .await;
        super::javascript_dialog::settle_pending_popup_dialogs(
            conn,
            out,
            &browser_context_id,
            popup_id,
            target_id.as_deref(),
        );
    }
}

fn resolve_devtools_opener(
    conn: &CdpConnection,
    page_owner: &TargetPageResidenceIdentity,
    source: &RendererPopupActivationSource,
) -> Option<PopupTargetOpenerIdentity> {
    let RendererPopupActivationSource::Window {
        root_document,
        window,
        ..
    } = source
    else {
        return None;
    };
    let browser_context = conn.browser_context_by_id(page_owner.browser_context_id())?;

    // CDP opener identity describes which target created this auxiliary
    // browsing context; it is not the DOM `window.opener` access grant.
    // Chromium keeps openerId/openerFrameId for implicit-noopener `_blank`
    // targets so automation clients can associate the popup with its page,
    // while independently reporting canAccessOpener=false.
    let resolved = match window {
        RendererWindowDocumentSource::RootFrame => {
            let target_id = page_owner.target_id()?;
            browser_context
                .devtools_target_info(target_id)
                .is_some()
                .then(|| PopupTargetOpenerIdentity::new(target_id, target_id))
        }
        RendererWindowDocumentSource::ChildFrame { frame_id, .. } => {
            let target_id = page_owner.target_id()?;
            browser_context
                .devtools_target_info(target_id)
                .is_some()
                .then(|| PopupTargetOpenerIdentity::new(target_id, frame_id))
        }
        RendererWindowDocumentSource::LightweightPopup { popup_id, .. } => {
            let target_id = browser_context.target_id_for_popup_id(*popup_id)?;
            Some(PopupTargetOpenerIdentity::new(target_id, target_id))
        }
    };
    if resolved.is_none() {
        tracing::debug!(
            browser_context_id = page_owner.browser_context_id(),
            target_id = page_owner.target_id(),
            page_attachment_id = page_owner.page_attachment_id().get(),
            ?root_document,
            ?window,
            "popup action retained after its exact opener browsing context disappeared"
        );
    }
    resolved
}

#[cfg(test)]
impl PagePreparedPopupActivation {
    pub(super) fn from_renderer_for_test(
        page_owner: TargetPageResidenceIdentity,
        activation: RendererPendingPopupActivation,
    ) -> Self {
        Self::new(page_owner, activation)
    }
}

#[cfg(test)]
mod tests {
    use moli_core::{
        PageId,
        page::{
            RendererDocumentLifecycleIdentity, RendererDocumentToken, RendererFrameToken,
            RendererLifecycleEpoch,
        },
    };

    use crate::conn::{BrowserContext, CdpConnection, TargetPageResidenceIdentity};

    use super::*;

    fn page_owner(browser_context_id: &str, target_id: &str) -> TargetPageResidenceIdentity {
        TargetPageResidenceIdentity::new_for_test(
            browser_context_id.to_owned(),
            Some(target_id.to_owned()),
            7,
        )
    }

    fn source_document() -> RendererDocumentLifecycleIdentity {
        let page_id = PageId::new_for_testing(1);
        RendererDocumentLifecycleIdentity {
            frame: RendererFrameToken { page_id },
            document: RendererDocumentToken::new_for_testing(page_id, 3),
            epoch: RendererLifecycleEpoch(5),
        }
    }

    fn context(browser_context_id: &str, target_id: &str, session_id: &str) -> BrowserContext {
        let mut context = BrowserContext::new(browser_context_id.to_owned());
        context.set_active_target_id(target_id);
        context.attach_active_session(session_id);
        context
    }

    fn window_activation(
        source: RendererWindowDocumentSource,
        exposes_opener: bool,
        popup_id: Option<u64>,
        url: &str,
    ) -> RendererPendingPopupActivation {
        RendererPendingPopupActivation::window(
            source_document(),
            source,
            exposes_opener,
            popup_id,
            url.to_owned(),
            "_blank".to_owned(),
        )
    }

    async fn emit(
        conn: &mut CdpConnection,
        owner: TargetPageResidenceIdentity,
        activations: Vec<RendererPendingPopupActivation>,
    ) {
        emit_prepared(
            conn,
            &mut Vec::new(),
            activations
                .into_iter()
                .map(|activation| {
                    PagePreparedPopupActivation::from_renderer_for_test(owner.clone(), activation)
                })
                .collect(),
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn popup_uses_captured_context_and_opener_after_another_context_becomes_active() {
        let mut conn = CdpConnection::default();
        conn.inactive_browser_contexts
            .push(context("BID-source", "TID-source", "SID-source"));
        conn.browser_context = Some(context("BID-current", "TID-current", "SID-current"));

        emit(
            &mut conn,
            page_owner("BID-source", "TID-source"),
            vec![window_activation(
                RendererWindowDocumentSource::RootFrame,
                true,
                Some(41),
                "about:blank#captured-context",
            )],
        )
        .await;

        let source = conn
            .browser_context_by_id("BID-source")
            .expect("captured browser context");
        let popup_target_id = source
            .target_id_for_popup_id(41)
            .expect("popup should be created in the captured context");
        let info = source
            .devtools_target_info(popup_target_id)
            .expect("captured popup target info");
        assert_eq!(
            info.opener_id.as_ref().map(|id| id.as_str()),
            Some("TID-source")
        );
        assert_eq!(
            info.opener_frame_id.as_ref().map(|id| id.as_str()),
            Some("TID-source")
        );
        assert!(
            conn.browser_context_by_id("BID-current")
                .expect("current browser context")
                .background_targets
                .is_empty(),
            "draining from another current session must not redirect the popup"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn noopener_popup_retains_devtools_creator_without_dom_opener_access() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(context("BID-1", "TID-opener", "SID-1"));

        emit(
            &mut conn,
            page_owner("BID-1", "TID-opener"),
            vec![window_activation(
                RendererWindowDocumentSource::RootFrame,
                false,
                Some(42),
                "about:blank#noopener",
            )],
        )
        .await;

        let context = conn.browser_context_by_id("BID-1").unwrap();
        assert_eq!(context.active_target_id(), Some("TID-opener"));
        let popup_target_id = context.target_id_for_popup_id(42).unwrap();
        let info = context.devtools_target_info(popup_target_id).unwrap();
        assert_eq!(
            info.opener_id.as_ref().map(|id| id.as_str()),
            Some("TID-opener")
        );
        assert_eq!(
            info.opener_frame_id.as_ref().map(|id| id.as_str()),
            Some("TID-opener")
        );
        assert!(!info.can_access_opener);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn removed_opener_downgrades_access_without_rebinding_to_current_target() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(context("BID-1", "TID-current", "SID-1"));

        emit(
            &mut conn,
            page_owner("BID-1", "TID-removed"),
            vec![window_activation(
                RendererWindowDocumentSource::RootFrame,
                true,
                Some(47),
                "about:blank#removed-opener",
            )],
        )
        .await;

        let context = conn.browser_context_by_id("BID-1").unwrap();
        let popup_target_id = context.target_id_for_popup_id(47).unwrap();
        let info = context.devtools_target_info(popup_target_id).unwrap();
        assert!(info.opener_id.is_none());
        assert!(info.opener_frame_id.is_none());
        assert!(!info.can_access_opener);
        assert_eq!(context.active_target_id(), Some("TID-current"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn child_window_popup_preserves_its_exact_opener_frame() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(context("BID-1", "TID-root", "SID-1"));

        emit(
            &mut conn,
            page_owner("BID-1", "TID-root"),
            vec![window_activation(
                RendererWindowDocumentSource::ChildFrame {
                    frame_id: "FRAME-child".to_owned(),
                    local_window_id: 9,
                    document_id: 11,
                },
                true,
                Some(43),
                "about:blank#child-opener",
            )],
        )
        .await;

        let context = conn.browser_context_by_id("BID-1").unwrap();
        let popup_target_id = context.target_id_for_popup_id(43).unwrap();
        let info = context.devtools_target_info(popup_target_id).unwrap();
        assert_eq!(
            info.opener_id.as_ref().map(|id| id.as_str()),
            Some("TID-root")
        );
        assert_eq!(
            info.opener_frame_id.as_ref().map(|id| id.as_str()),
            Some("FRAME-child")
        );
        assert!(info.can_access_opener);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fifo_popup_batch_resolves_a_lightweight_popup_as_the_next_opener() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(context("BID-1", "TID-root", "SID-1"));
        let owner = page_owner("BID-1", "TID-root");

        emit(
            &mut conn,
            owner,
            vec![
                window_activation(
                    RendererWindowDocumentSource::RootFrame,
                    true,
                    Some(44),
                    "about:blank#first",
                ),
                window_activation(
                    RendererWindowDocumentSource::LightweightPopup {
                        popup_id: 44,
                        popup_document_id: 12,
                    },
                    true,
                    Some(45),
                    "about:blank#second",
                ),
            ],
        )
        .await;

        let context = conn.browser_context_by_id("BID-1").unwrap();
        let first_target_id = context.target_id_for_popup_id(44).unwrap();
        let second_target_id = context.target_id_for_popup_id(45).unwrap();
        let second = context.devtools_target_info(second_target_id).unwrap();
        assert_eq!(
            second.opener_id.as_ref().map(|id| id.as_str()),
            Some(first_target_id)
        );
        assert_eq!(
            second.opener_frame_id.as_ref().map(|id| id.as_str()),
            Some(first_target_id)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn removed_captured_context_does_not_fall_back_to_the_active_context() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(context("BID-current", "TID-current", "SID-current"));

        emit(
            &mut conn,
            page_owner("BID-removed", "TID-removed"),
            vec![RendererPendingPopupActivation::browser_context(
                Some(46),
                "about:blank#removed-context".to_owned(),
                "_blank".to_owned(),
            )],
        )
        .await;

        assert!(
            conn.browser_context
                .as_ref()
                .expect("current browser context")
                .background_targets
                .is_empty()
        );
    }
}
