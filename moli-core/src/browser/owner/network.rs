use super::Browser;
use crate::browser::{BrowserContextId, BrowserEvent, BrowserSequence, NetworkOccurrence};
use crate::page::{RendererNetworkInput, RendererNetworkOutputItem, ScriptNetworkOutputItem};

impl Browser {
    pub(super) fn commit_network(&mut self, id: BrowserContextId, input: RendererNetworkInput) {
        let Ok(context) = self.context_mut(id) else {
            return;
        };
        let input = match input {
            RendererNetworkInput::Observation(input) => input,
            RendererNetworkInput::SourceClosed {
                runtime,
                owner_local_host_id,
                page,
            } => {
                if context.routes_renderer_browser_context_runtime(runtime)
                    && let Some(document) = context.network_requests.close_source(
                        crate::browser::RendererPageResidenceIdentity::from_parts(
                            owner_local_host_id,
                            page,
                        ),
                    )
                {
                    self.events
                        .publish(BrowserEvent::NetworkSourceClosed(document));
                }
                return;
            }
        };
        let occurrence = &input.occurrence;
        if !context.routes_renderer_browser_context_runtime(occurrence.runtime) {
            return;
        }
        let admitted = crate::browser::network::request_key(occurrence)
            .as_ref()
            .and_then(|key| context.network_requests.get(key));
        let renderer_document =
            admitted.map_or(occurrence.document, |entry| entry.renderer_document);
        let document = admitted.map(|entry| entry.document).or_else(|| {
            context.network_document_for_renderer(
                crate::browser::RendererPageResidenceIdentity::from_parts(
                    occurrence.owner_local_host_id,
                    occurrence.document.document.page_id,
                ),
            )
        });
        let Some(document) = document else {
            return;
        };
        let sequence = BrowserSequence::allocate();
        if !context
            .network_requests
            .commit(document, occurrence, sequence)
        {
            return;
        }
        let mut renderer = occurrence.clone();
        if renderer.document != renderer_document {
            std::sync::Arc::make_mut(&mut renderer).document = renderer_document;
        }
        let event = NetworkOccurrence { document, renderer };
        let event = match &occurrence.item {
            RendererNetworkOutputItem::ChildDocument(_) => {
                BrowserEvent::NetworkRequestCompleted(event)
            }
            RendererNetworkOutputItem::Resource(item) => match item.as_ref() {
                ScriptNetworkOutputItem::SubresourceRequestStarted(_) => {
                    BrowserEvent::NetworkRequestStarted(event)
                }
                ScriptNetworkOutputItem::SubresourceNetworkRecord(_)
                | ScriptNetworkOutputItem::SubresourceBodyFinished(_) => {
                    BrowserEvent::NetworkRequestCompleted(event)
                }
                _ => BrowserEvent::NetworkActivity(event),
            },
        };
        // State, semantic event and the concrete FIFO's receipt are committed in
        // this one owner turn. Draining Protocol can only observe the result.
        self.events.publish_committed(sequence, event);
        input.commit(sequence.get(), renderer_document);
    }
}
