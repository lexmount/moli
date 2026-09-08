use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use tokio::sync::watch;

use super::{Browser, BrowserHandle, WebContentsCreation};
use crate::browser::{
    BrowserEvent, BrowserPopupAdmission, BrowserPopupCreation, DocumentHandle,
    RendererPageResidenceIdentity,
    web_contents::{InitialDocumentCreator, SessionStorageNamespace},
};
use crate::page::{
    RendererPendingPopupActivation, RendererPopupActivationSource, RendererPopupDisposition,
    RendererPopupOpening, RendererPopupOpeningId, RendererWindowDocumentSource,
};

struct PopupAdmissionRecord {
    opening: Weak<RendererPopupOpening>,
    result: Option<BrowserPopupAdmission>,
}

pub(super) struct PopupAdmissions {
    records: HashMap<RendererPopupOpeningId, PopupAdmissionRecord>,
    changed: watch::Sender<()>,
}

impl Default for PopupAdmissions {
    fn default() -> Self {
        Self {
            records: HashMap::new(),
            changed: watch::channel(()).0,
        }
    }
}

#[derive(Clone)]
struct PopupSource {
    document: DocumentHandle,
    renderer: RendererPageResidenceIdentity,
    creator: Option<InitialDocumentCreator>,
}

impl Browser {
    pub(super) fn observe_popup_inputs(&mut self, document: DocumentHandle) {
        let Ok(context) = self.context(document.web_contents().context()) else {
            return;
        };
        let Ok(host) = context.document(document) else {
            return;
        };
        let creator = host
            .commit
            .as_ref()
            .and_then(|commit| commit.info.as_ref())
            .map(|info| {
                InitialDocumentCreator::new(
                    document.web_contents().id(),
                    info.security_origin.clone(),
                    info.secure_context_type.clone(),
                )
            })
            .or_else(|| {
                context
                    .web_contents_initial_document_state(document.web_contents())
                    .ok()
                    .flatten()
                    .and_then(|initial| initial.creator().cloned())
            });
        let source = PopupSource {
            document,
            renderer: RendererPageResidenceIdentity::from_page(&host.page),
            creator,
        };
        let mut inputs = host.page.observe_popup_inputs();
        let sender = self.local_sender.clone();
        tokio::task::spawn_local(async move {
            // Unlike lifecycle/dialog state, accepted auxiliary contexts survive
            // their source Document's retirement. Page close seals admission and
            // this receiver drains its already-accepted tail before stopping.
            while let Some(requests) = inputs.recv().await {
                let source = source.clone();
                let (committed, completion) = tokio::sync::oneshot::channel();
                if sender
                    .send(Box::new(move |browser| {
                        browser.admit_popups(&source, requests);
                        let _ = committed.send(());
                    }))
                    .is_err()
                    || completion.await.is_err()
                {
                    break;
                }
            }
        });
    }

    fn admit_popups(
        &mut self,
        source: &PopupSource,
        requests: Vec<RendererPendingPopupActivation>,
    ) {
        // One sweep per concrete batch, not quadratic work for a large script
        // turn. Only live observations and the latest batch retain receipts.
        self.popup_admissions
            .records
            .retain(|_, record| record.opening.strong_count() != 0);
        for request in requests {
            self.admit_popup(source, request);
        }
    }

    fn admit_popup(&mut self, source: &PopupSource, request: RendererPendingPopupActivation) {
        let (opening, storage, storage_key) = request.into_parts();
        let result = (|| {
            let context = self.context_mut(source.document.web_contents().context())?;
            let reusable_name = (!opening.target_name().is_empty()
                && !opening.target_name().eq_ignore_ascii_case("_blank"))
            .then(|| opening.target_name().to_owned());
            let existing = reusable_name
                .as_deref()
                .and_then(|name| context.web_contents_handle_for_window_name(name));
            let (contents, created) = if let Some(contents) = existing {
                (contents, false)
            } else {
                let source_window = match opening.source() {
                    RendererPopupActivationSource::Window { window, .. } => Some(window.clone()),
                    RendererPopupActivationSource::BrowserContext => None,
                };
                let opener = match source_window.as_ref() {
                    Some(
                        RendererWindowDocumentSource::RootFrame
                        | RendererWindowDocumentSource::ChildFrame { .. },
                    ) => context
                        .contains_web_contents(source.document.web_contents())
                        .then_some(source.document.web_contents()),
                    Some(RendererWindowDocumentSource::LightweightPopup { popup_id, .. }) => {
                        context.web_contents_for_renderer_popup(source.renderer, *popup_id)
                    }
                    None => None,
                };
                let can_access = opener.is_some()
                    && matches!(
                        opening.source(),
                        RendererPopupActivationSource::Window {
                            exposes_opener: true,
                            ..
                        }
                    );
                let creator = can_access
                    .then(|| source.creator.clone())
                    .flatten()
                    .map(|creator| {
                        InitialDocumentCreator::new(
                            opener.unwrap().id(),
                            creator.security_origin().to_owned(),
                            creator.secure_context_type().to_owned(),
                        )
                    });
                let namespace = storage
                    .map(SessionStorageNamespace::from_store)
                    .or_else(|| {
                        can_access
                            .then(|| context.clone_session_storage_namespace(opener.unwrap().id()))
                            .flatten()
                    });
                let mut creation = WebContentsCreation::with_initial_document(
                    "about:blank".into(),
                    creator,
                    storage_key,
                );
                if let Some(namespace) = namespace {
                    creation = creation.with_session_storage(namespace);
                }
                let (contents, _) = context.register_web_contents(creation.build())?;
                context.set_web_contents_window_name(contents, reusable_name)?;
                context.set_web_contents_opener(contents, opener, can_access)?;
                context.web_contents_mut(contents)?.window.popup_creation =
                    Some(Arc::new(BrowserPopupCreation {
                        request: opening.id(),
                        source_document: source.document,
                        source_renderer: source.renderer,
                        source_window,
                        opener,
                        requested_url: opening.url().to_owned(),
                        renderer_opening: Arc::downgrade(&opening),
                    }));
                (contents, true)
            };
            if let Some(popup_id) = opening.popup_id() {
                let aliases = &mut context
                    .web_contents_mut(contents)?
                    .window
                    .renderer_popup_sources;
                let alias = (source.renderer, popup_id);
                if !aliases.contains(&alias) {
                    aliases.push(alias);
                }
            }
            Ok::<_, String>(BrowserPopupAdmission {
                source_document: source.document,
                web_contents: contents,
                created,
            })
        })();
        match &result {
            Ok(admission) => {
                if admission.created {
                    self.events
                        .publish(BrowserEvent::WebContentsCreated(admission.web_contents));
                }
                if opening.disposition() == RendererPopupDisposition::Foreground {
                    let _ = self.activate_web_contents(admission.web_contents);
                }
            }
            Err(error) => {
                tracing::debug!(%error, "accepted popup's native Context is no longer available")
            }
        }
        // Retain correlation only while the concrete renderer observation lives.
        // This is not another Window registry or an unbounded event history.
        self.popup_admissions.records.insert(
            opening.id(),
            PopupAdmissionRecord {
                opening: Arc::downgrade(&opening),
                result: result.ok(),
            },
        );
        self.popup_admissions.changed.send_modify(|_| {});
    }
}

#[cfg(test)]
mod tests;

impl BrowserHandle {
    /// Observe admission of an already accepted renderer input. This cannot
    /// create/reuse a Window, choose an opener, or reselect a current Document.
    pub async fn wait_for_renderer_popup(
        &self,
        opening: Arc<RendererPopupOpening>,
    ) -> Option<BrowserPopupAdmission> {
        let mut changed = self
            .execute(|browser| browser.popup_admissions.changed.subscribe())
            .ok()?;
        loop {
            changed.borrow_and_update();
            let id = opening.id();
            if let Some(result) = self
                .execute(move |browser| {
                    browser
                        .popup_admissions
                        .records
                        .get(&id)
                        .map(|record| record.result)
                })
                .ok()?
            {
                return result;
            }
            changed.changed().await.ok()?;
        }
    }
}
