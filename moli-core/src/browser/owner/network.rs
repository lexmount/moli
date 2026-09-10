use super::Browser;
use crate::browser::{
    BrowserContextId, BrowserEvent, BrowserSequence, NetworkOccurrence, NetworkOwner, WorkerHandle,
};
use crate::page::{
    RendererNetworkInput, RendererNetworkOutputItem, RendererNetworkSource,
    RendererWorkerNetworkSource, ScriptNetworkOutputItem,
};

impl Browser {
    pub(super) fn commit_network(&mut self, id: BrowserContextId, input: RendererNetworkInput) {
        let Ok(context) = self.context_mut(id) else {
            return;
        };
        let input = match input {
            RendererNetworkInput::Observation(input) => input,
            RendererNetworkInput::SourceClosed { runtime, source } => {
                if context.routes_renderer_browser_context_runtime(runtime)
                    && let Some(owner) = context.network_requests.close_source(&source)
                {
                    self.events
                        .publish(BrowserEvent::NetworkSourceClosed { owner, source });
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
        let renderer_source = admitted.map_or_else(
            || occurrence.source.clone(),
            |entry| entry.renderer_source.clone(),
        );
        let owner = admitted
            .map(|entry| entry.owner)
            .or_else(|| match &occurrence.source {
                RendererNetworkSource::Document {
                    owner_local_host_id,
                    document,
                } => context
                    .network_document_for_renderer(
                        crate::browser::RendererPageResidenceIdentity::from_parts(
                            *owner_local_host_id,
                            document.document.page_id,
                        ),
                    )
                    .map(NetworkOwner::Document),
                RendererNetworkSource::Worker(worker) => {
                    let handle = match worker {
                        RendererWorkerNetworkSource::Shared(instance) => {
                            context.shared_workers.get(instance)?;
                            WorkerHandle::Shared {
                                context: id,
                                instance: *instance,
                            }
                        }
                        RendererWorkerNetworkSource::Service { version, run } => {
                            if context.service_workers.get(version)?.execution.active_run()
                                != Some(run)
                            {
                                return None;
                            }
                            WorkerHandle::Service {
                                context: id,
                                version: *version,
                            }
                        }
                    };
                    Some(NetworkOwner::Worker(handle))
                }
            });
        let Some(owner) = owner else {
            return;
        };
        let sequence = BrowserSequence::allocate();
        if !context.network_requests.commit(owner, occurrence, sequence) {
            return;
        }
        let mut renderer = occurrence.clone();
        if renderer.source != renderer_source {
            std::sync::Arc::make_mut(&mut renderer).source = renderer_source.clone();
        }
        let event = NetworkOccurrence { owner, renderer };
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
        input.commit(sequence.get(), renderer_source);
    }
}
